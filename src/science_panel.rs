//! Client-side Science Panel plugin.
//!
//! Owns the Science console UI: long-range radar overlay, system chart,
//! science target designation, cancel-impulse button, and view-mode controls.
//!
//! Extracted from `client/app.rs` as part of the "Client split" series.
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client_app::OutboundClientMessage;
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::set_science_target_message;
use crate::messages::{ClientMessage, Console, GamePhase, ViewMode};

// ── Pure visibility helper ────────────────────────────────────────────

/// Decide whether the science panel should be visible.
///
/// Rules:
/// 1. Game phase must be `InProgress`.
/// 2. The local player must hold `Console::Sensors`.
/// 3. If holding **one console only**, show automatically.
/// 4. If holding **multiple consoles**, show only when `ActiveConsole`
///    is explicitly set to `Sensors`.
pub fn science_panel_visible(
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

// ── Resources ────────────────────────────────────────────────────────

/// Tracks which sub-view the Science console is currently showing.
#[derive(Resource, Clone, Debug, PartialEq, Eq, Default)]
pub enum ScienceView {
    /// Long-range radar — shows entities within `SCIENCE_RADAR_RANGE`.
    #[default]
    ScienceRadar,
    /// System chart — shows navigational entities (stars, planets, fields).
    SystemChart,
}

// ── Marker components ────────────────────────────────────────────────

/// Marks the root of the Science console UI; shown only when the local
/// player holds `Console::Sensors` and the phase is InProgress.
#[derive(Component)]
pub struct SciencePanel;

/// Marks the "Science Radar" view-mode button on the Science console.
#[derive(Component)]
pub struct ScienceRadarButton;

/// Marks the "System Chart" view-mode button on the Science console.
#[derive(Component)]
pub struct ScienceSystemChartButton;

/// Marks the "Cancel Impulse" button on the Science console.
#[derive(Component)]
pub struct ScienceCancelImpulseButton;

/// Marks the "On Screen" button on the Science console.
#[derive(Component)]
pub struct ScienceOnScreenButton;

// ── Plugin ────────────────────────────────────────────────────────────

pub struct SciencePanelPlugin;

impl Plugin for SciencePanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ScienceView>()
            .add_systems(Startup, setup_science_ui)
            .add_systems(
                Update,
                (
                    toggle_science_panel_visibility,
                    handle_science_radar_button_press,
                    handle_science_system_chart_button_press,
                    handle_science_cancel_impulse_button_press,
                    handle_science_on_screen_button_press,
                ),
            );
    }
}

// ── Setup ────────────────────────────────────────────────────────────

fn setup_science_ui(mut commands: Commands) {
    commands.spawn((
        SciencePanel,
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
            Text::new("Science"),
            TextFont { font_size: 32.0, ..default() },
            TextColor(Color::srgb(0.4, 1.0, 0.8)),
        ));

        // View-mode selector row
        panel.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                ScienceRadarButton,
                Button,
                Node {
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.10, 0.30, 0.25)),
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("Radar"),
                    TextFont { font_size: 13.0, ..default() },
                    TextColor(Color::srgb(0.4, 1.0, 0.8)),
                ));
            });

            row.spawn((
                ScienceSystemChartButton,
                Button,
                Node {
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.10, 0.25, 0.30)),
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("System Chart"),
                    TextFont { font_size: 13.0, ..default() },
                    TextColor(Color::srgb(0.4, 0.8, 1.0)),
                ));
            });
        });

        // On Screen button
        panel.spawn((
            ScienceOnScreenButton,
            Button,
            Node {
                padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.10, 0.30, 0.15)),
        ))
        .with_children(|btn| {
            btn.spawn((
                Text::new("ON SCREEN"),
                TextFont { font_size: 14.0, ..default() },
                TextColor(Color::srgb(0.5, 1.0, 0.5)),
            ));
        });

        // Cancel Impulse button
        panel.spawn((
            ScienceCancelImpulseButton,
            Button,
            Node {
                padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.40, 0.05, 0.05)),
        ))
        .with_children(|btn| {
            btn.spawn((
                Text::new("CANCEL IMPULSE"),
                TextFont { font_size: 14.0, ..default() },
                TextColor(Color::srgb(1.0, 0.4, 0.4)),
            ));
        });
    });
}

// ── Systems ──────────────────────────────────────────────────────────

fn toggle_science_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<SciencePanel>>,
) {
    if !lobby.is_changed() && !token.is_changed() && !active.is_changed() {
        return;
    }
    let visible = science_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

fn handle_science_radar_button_press(
    interactions: Query<&Interaction, (Changed<Interaction>, With<Button>, With<ScienceRadarButton>)>,
    mut science_view: ResMut<ScienceView>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction == Interaction::Pressed {
            *science_view = ScienceView::ScienceRadar;
            outbound.write(OutboundClientMessage(
                ClientMessage::SetView { mode: ViewMode::ScienceRadar },
            ));
        }
    }
}

fn handle_science_system_chart_button_press(
    interactions: Query<&Interaction, (Changed<Interaction>, With<Button>, With<ScienceSystemChartButton>)>,
    mut science_view: ResMut<ScienceView>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction == Interaction::Pressed {
            *science_view = ScienceView::SystemChart;
            outbound.write(OutboundClientMessage(
                ClientMessage::SetView { mode: ViewMode::SystemChart },
            ));
        }
    }
}

fn handle_science_cancel_impulse_button_press(
    interactions: Query<&Interaction, (Changed<Interaction>, With<Button>, With<ScienceCancelImpulseButton>)>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(ClientMessage::CancelImpulse));
        }
    }
}

fn handle_science_on_screen_button_press(
    interactions: Query<&Interaction, (Changed<Interaction>, With<Button>, With<ScienceOnScreenButton>)>,
    science_view: Res<ScienceView>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction == Interaction::Pressed {
            let mode = match *science_view {
                ScienceView::ScienceRadar => ViewMode::ScienceRadar,
                ScienceView::SystemChart => ViewMode::SystemChart,
            };
            outbound.write(OutboundClientMessage(ClientMessage::SetView { mode }));
        }
    }
}

/// Build the `ClientMessage` for designating a science target.
pub fn science_target_message(uuid: String) -> ClientMessage {
    set_science_target_message(uuid)
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

    // ── science_panel_visible ─────────────────────────────────────────

    #[test]
    fn science_panel_not_visible_in_lobby_phase() {
        let lobby = LobbyState::default(); // Lobby phase
        let active = ActiveConsole::default();
        assert!(!science_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn science_panel_not_visible_when_player_does_not_hold_sensors() {
        let lobby = lobby_in_progress();
        let active = ActiveConsole::default();
        assert!(!science_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn science_view_default_is_science_radar() {
        let v = ScienceView::default();
        assert_eq!(v, ScienceView::ScienceRadar);
    }

    // ── science_target_message ────────────────────────────────────────

    #[test]
    fn science_target_message_produces_set_science_target() {
        let msg = science_target_message("entity-42".into());
        assert_eq!(msg, ClientMessage::SetScienceTarget { uuid: "entity-42".into() });
    }
}
