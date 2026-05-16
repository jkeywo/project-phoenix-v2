//! Client-side Science (Sensors) Panel plugin.
//!
//! Owns the Sensors console UI: long-range radar display, science target
//! designation, cancel-impulse button (visible only at impulse), and
//! view-mode controls.
//!
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client_app::OutboundClientMessage;
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::set_science_target_message;
use crate::messages::{ClientMessage, Console, GamePhase, ViewMode};
use crate::ship_view::ShipView;

// ── Pure visibility helper ────────────────────────────────────────────

/// Decide whether the science panel should be visible.
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

// ── Marker components ────────────────────────────────────────────────

/// Marks the root of the Science console UI.
#[derive(Component)]
pub struct SciencePanel;

/// Marks the long-range radar display panel (gizmo-drawn).
#[derive(Component)]
pub struct ScienceRadarPanel;

/// Marks the "Radar" view-mode button.
#[derive(Component)]
pub struct ScienceRadarButton;

/// Marks the "Cancel Impulse" button.
#[derive(Component)]
pub struct ScienceCancelImpulseButton;

/// Marks the "On Screen" button.
#[derive(Component)]
pub struct ScienceOnScreenButton;

// ── Constants ─────────────────────────────────────────────────────────

const CANCEL_IMPULSE_BG: Color = Color::srgb(0.40, 0.05, 0.05);
const CANCEL_IMPULSE_TEXT: Color = Color::srgb(1.0, 0.4, 0.4);

// Gizmo radar colours (same palette as weapons radar).
const RADAR_OUTER_RING_COLOR: Color = Color::srgb(0.55, 0.70, 1.0);
const RADAR_MID_RING_COLOR:   Color = Color::srgb(0.30, 0.40, 0.65);
const RADAR_ASTEROID_COLOR:   Color = Color::srgb(0.85, 0.75, 0.45);
const RADAR_SHIP_COLOR:       Color = Color::srgb(0.95, 0.95, 1.0);

// ── Plugin ────────────────────────────────────────────────────────────

pub struct SciencePanelPlugin;

impl Plugin for SciencePanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_science_ui)
            .add_systems(
                Update,
                (
                    toggle_science_panel_visibility,
                    refresh_cancel_impulse_visibility,
                    handle_science_radar_button_press,
                    handle_science_cancel_impulse_button_press,
                    handle_science_on_screen_button_press,
                    draw_science_radar,
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
            Text::new("Sensors"),
            TextFont { font_size: 32.0, ..default() },
            TextColor(Color::srgb(0.8, 0.8, 1.0)),
        ));

        // Long-range radar display (gizmo-drawn via ScienceRadarPanel bounds)
        panel.spawn((
            ScienceRadarPanel,
            Node {
                width:  Val::Px(240.0),
                height: Val::Px(240.0),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BorderColor::all(Color::srgb(0.55, 0.70, 1.0)),
            BackgroundColor(Color::srgb(0.04, 0.06, 0.12)),
        ));

        // Radar view-mode button — pushes ScienceRadar to the viewscreen
        panel.spawn((
            ScienceRadarButton,
            Button,
            Node {
                padding: UiRect::axes(Val::Px(18.0), Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.10, 0.30, 0.25)),
        ))
        .with_children(|btn| {
            btn.spawn((
                Text::new("ON SCREEN"),
                TextFont { font_size: 14.0, ..default() },
                TextColor(Color::srgb(0.4, 1.0, 0.8)),
            ));
        });

        // Cancel Impulse button — starts hidden; shown only when impulse is active
        panel.spawn((
            ScienceCancelImpulseButton,
            Button,
            Node {
                padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(CANCEL_IMPULSE_BG),
            Visibility::Hidden,
        ))
        .with_children(|btn| {
            btn.spawn((
                Text::new("CANCEL IMPULSE"),
                TextFont { font_size: 14.0, ..default() },
                TextColor(CANCEL_IMPULSE_TEXT),
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

/// Toggle Cancel Impulse button visibility based on impulse charge progress.
fn refresh_cancel_impulse_visibility(
    ship_view: Res<ShipView>,
    mut buttons: Query<&mut Visibility, With<ScienceCancelImpulseButton>>,
) {
    for mut vis in buttons.iter_mut() {
        *vis = if ship_view.impulse_charge_progress > 0.0 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

fn handle_science_radar_button_press(
    interactions: Query<&Interaction, (Changed<Interaction>, With<Button>, With<ScienceRadarButton>)>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(
                ClientMessage::SetView { mode: ViewMode::ScienceRadar },
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

/// On Screen button — always pushes ScienceRadar to the viewscreen.
fn handle_science_on_screen_button_press(
    interactions: Query<&Interaction, (Changed<Interaction>, With<Button>, With<ScienceOnScreenButton>)>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(
                ClientMessage::SetView { mode: ViewMode::ScienceRadar },
            ));
        }
    }
}

/// Draw the Science long-range radar on the `ScienceRadarPanel` using gizmos.
fn draw_science_radar(
    mut gizmos: Gizmos,
    panel: Query<(&ComputedNode, &GlobalTransform, &ViewVisibility), With<ScienceRadarPanel>>,
    science_panel: Query<&Visibility, With<SciencePanel>>,
    sim: Res<crate::client_sim::ClientSimState>,
    ship_view: Res<ShipView>,
    windows: Query<&Window>,
) {
    if !science_panel
        .iter()
        .any(|v| matches!(v, Visibility::Visible | Visibility::Inherited))
    {
        return;
    }
    let Ok((node, gt, view_vis)) = panel.single() else { return };
    if !view_vis.get() {
        return;
    }
    let Ok(window) = windows.single() else { return };
    let viewport_w = window.width();
    let viewport_h = window.height();

    let node_size = node.size();
    let node_centre_screen = gt.translation().truncate();
    let centre_world_x = node_centre_screen.x - viewport_w / 2.0;
    let centre_world_y = viewport_h / 2.0 - node_centre_screen.y;
    let centre = Vec2::new(centre_world_x, centre_world_y);

    let radius = node_size.x.min(node_size.y) * 0.5;
    if radius <= 0.0 {
        return;
    }

    // Draw outer and mid range rings
    gizmos.circle_2d(centre, radius, RADAR_OUTER_RING_COLOR);
    let range = crate::client_sim::science_radar_config().range;
    let mid_ratio = crate::radar::RADAR_MID_RING / range;
    gizmos.circle_2d(centre, radius * mid_ratio, RADAR_MID_RING_COLOR);

    // Compute the long-range radar view
    let radar_view = crate::client_sim::compute_science_long_range_radar_view(&sim, &ship_view);
    for dot in &radar_view.dots {
        let pos = centre + Vec2::new(dot.radar_x * radius, dot.radar_y * radius);
        let pix_radius = (dot.scaled_radius * radius).max(2.0);
        gizmos.circle_2d(pos, pix_radius, RADAR_ASTEROID_COLOR);
    }

    // Draw rings (asteroid fields, regions)
    for ring in &radar_view.rings {
        let pos = centre + Vec2::new(ring.centre_x * radius, ring.centre_y * radius);
        let outer_r = ring.outer_r * radius;
        gizmos.circle_2d(pos, outer_r, Color::srgb(0.3, 0.7, 0.4));
        let inner_r = ring.inner_r * radius;
        if inner_r > 0.0 {
            gizmos.circle_2d(pos, inner_r, Color::srgb(0.2, 0.5, 0.3));
        }
    }

    // Ship triangle at centre
    let nose_len  = radius * 0.10;
    let half_base = radius * 0.06;
    let nose  = centre + Vec2::new(0.0,  nose_len);
    let left  = centre + Vec2::new(-half_base, -nose_len * 0.6);
    let right = centre + Vec2::new( half_base, -nose_len * 0.6);
    gizmos.line_2d(nose, left,  RADAR_SHIP_COLOR);
    gizmos.line_2d(left, right, RADAR_SHIP_COLOR);
    gizmos.line_2d(right, nose, RADAR_SHIP_COLOR);
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
        let lobby = LobbyState::default();
        let active = ActiveConsole::default();
        assert!(!science_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn science_panel_not_visible_when_player_does_not_hold_sensors() {
        let lobby = lobby_in_progress();
        let active = ActiveConsole::default();
        assert!(!science_panel_visible(&lobby, "tok", &active));
    }

    // ── science_target_message ────────────────────────────────────────

    #[test]
    fn science_target_message_produces_set_science_target() {
        let msg = science_target_message("entity-42".into());
        assert_eq!(msg, ClientMessage::SetScienceTarget { uuid: "entity-42".into() });
    }
}
