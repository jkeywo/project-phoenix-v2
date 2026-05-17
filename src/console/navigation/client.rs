//! Client-side Navigation Panel plugin.
//!
//! Owns the Navigation console UI: system chart display (gizmo-drawn),
//! impulse status text, cancel-impulse button, and on-screen viewscreen control.
//!
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client_app::OutboundClientMessage;
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::messages::{ClientMessage, Console, GamePhase, ViewMode};
use crate::ship_view::ShipView;

// ── Marker components ────────────────────────────────────────────────

/// Marks the root of the Navigation console UI.
#[derive(Component)]
pub struct NavigationPanel;

/// Marks the On Screen button; pressing it sends `SetView { NavigationChart }`.
#[derive(Component)]
pub struct NavOnScreenButton;

/// Marks the Cancel Impulse button on the Navigation console.
#[derive(Component)]
pub struct NavCancelImpulseButton;

/// Marks the impulse status text node.
#[derive(Component)]
pub struct NavImpulseStatusText;

/// Marks the navigation chart display container (gizmo-drawn).
#[derive(Component)]
pub struct NavChartPanel;

// ── Plugin ────────────────────────────────────────────────────────────

pub struct NavigationPanelPlugin;

impl Plugin for NavigationPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_navigation_ui)
            .add_systems(
                Update,
                (
                    toggle_navigation_panel_visibility,
                    refresh_navigation_panel,
                    handle_nav_on_screen_button_press,
                    handle_nav_cancel_impulse_button_press,
                    draw_nav_chart,
                ),
            );
    }
}

// ── Setup ────────────────────────────────────────────────────────────

fn setup_navigation_ui(mut commands: Commands) {
    commands.spawn((
        NavigationPanel,
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
            Text::new("Navigation"),
            TextFont { font_size: 32.0, ..default() },
            TextColor(Color::srgb(0.5, 1.0, 0.8)),
        ));

        // System chart display (gizmo-drawn via NavChartPanel bounds)
        panel.spawn((
            NavChartPanel,
            Node {
                width:  Val::Px(240.0),
                height: Val::Px(240.0),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BorderColor::all(Color::srgb(0.3, 1.0, 0.5)),
            BackgroundColor(Color::srgb(0.04, 0.06, 0.10)),
        ));

        // Impulse status row
        panel.spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(12.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                NavImpulseStatusText,
                Text::new("Impulse: Idle"),
                TextFont { font_size: 16.0, ..default() },
                TextColor(Color::srgb(0.5, 1.0, 0.8)),
            ));
            row.spawn((
                NavCancelImpulseButton,
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

        // On Screen button
        panel.spawn((
            NavOnScreenButton,
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
    });
}

// ── Systems ──────────────────────────────────────────────────────────

fn toggle_navigation_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<NavigationPanel>>,
) {
    let view = LobbyView::new(&lobby, &token.0);
    let holds = lobby.phase == GamePhase::InProgress
        && view.my_consoles().contains(&Console::Navigation);
    let my_consoles_count = view.my_consoles().len();
    let tab_active = match &active.0 {
        Some(c) => *c == Console::Navigation,
        None => my_consoles_count == 1,
    };
    let visible = holds && tab_active;
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

fn refresh_navigation_panel(
    ship_view: Res<ShipView>,
    mut status_text: Query<&mut Text, With<NavImpulseStatusText>>,
    mut cancel_btn: Query<&mut Visibility, With<NavCancelImpulseButton>>,
) {
    if !ship_view.is_changed() {
        return;
    }
    let charge = ship_view.impulse_charge_progress;
    for mut text in status_text.iter_mut() {
        let label = if charge >= 1.0 {
            "Impulse: ACTIVE"
        } else if charge > 0.0 {
            "Impulse: Charging"
        } else {
            "Impulse: Idle"
        };
        **text = label.to_string();
    }
    for mut vis in cancel_btn.iter_mut() {
        *vis = if charge > 0.0 { Visibility::Visible } else { Visibility::Hidden };
    }
}

fn handle_nav_on_screen_button_press(
    mut interactions: Query<
        &Interaction,
        (Changed<Interaction>, With<Button>, With<NavOnScreenButton>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(
                ClientMessage::SetView { mode: ViewMode::NavigationChart },
            ));
        }
    }
}

fn handle_nav_cancel_impulse_button_press(
    mut interactions: Query<
        &Interaction,
        (Changed<Interaction>, With<Button>, With<NavCancelImpulseButton>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(ClientMessage::CancelImpulse));
        }
    }
}

/// Draw the Navigation system chart on the `NavChartPanel` using gizmos.
fn draw_nav_chart(
    mut gizmos: Gizmos,
    panel: Query<(&ComputedNode, &GlobalTransform, &ViewVisibility), With<NavChartPanel>>,
    nav_panel: Query<&Visibility, With<NavigationPanel>>,
    sim: Res<ClientSimState>,
    ship_view: Res<ShipView>,
    windows: Query<&Window>,
) {
    if !nav_panel
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

    let chart_view = crate::client_sim::compute_system_chart_view(&sim, &ship_view);
    const ZOOM: f32 = 1.0;

    for ring in &chart_view.rings {
        let pos = centre + Vec2::new(ring.centre_x * radius / ZOOM, ring.centre_y * radius / ZOOM);
        let outer_r = ring.outer_r * radius / ZOOM;
        gizmos.circle_2d(pos, outer_r, Color::srgb(0.3, 0.7, 0.4));
        let inner_r = ring.inner_r * radius / ZOOM;
        if inner_r > 0.0 {
            gizmos.circle_2d(pos, inner_r, Color::srgb(0.2, 0.5, 0.3));
        }
    }

    for dot in &chart_view.dots {
        let pos = centre + Vec2::new(dot.radar_x * radius / ZOOM, dot.radar_y * radius / ZOOM);
        let pix_radius = (dot.scaled_radius * radius / ZOOM).max(3.0);
        gizmos.circle_2d(pos, pix_radius, Color::srgb(0.8, 0.8, 0.4));
    }

    let nose_len  = radius * 0.10;
    let half_base = radius * 0.06;
    let nose  = centre + Vec2::new(0.0,  nose_len);
    let left  = centre + Vec2::new(-half_base, -nose_len * 0.6);
    let right = centre + Vec2::new( half_base, -nose_len * 0.6);
    gizmos.line_2d(nose, left,  Color::srgb(0.5, 1.0, 0.8));
    gizmos.line_2d(left, right, Color::srgb(0.5, 1.0, 0.8));
    gizmos.line_2d(right, nose, Color::srgb(0.5, 1.0, 0.8));
}
