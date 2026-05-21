//! Client-side Navigation Panel plugin — migrated to `src/gui/` library widgets.
//!
//! Owns the Navigation console UI: system chart display (gizmo-drawn),
//! impulse status text readout (`TextReadout`), cancel-impulse button
//! (`GuiButton`), and on-screen viewscreen control (`GuiButton`).
//!
//! All callbacks are wired via observers at spawn time.
//! No per-button marker-component query systems remain.
//!
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client::console_shell::ConsoleShell;
use crate::client_app::OutboundClientMessage;
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::gui::{
    spawn_gui_button, ButtonPressed, ButtonSize, ReadoutValue, StateVisuals, TextReadout,
};
use crate::messages::{ClientMessage, Console, GamePhase, ViewMode};
use crate::phone_border::framing::{DeviceOrientation, PhoneAssets};
use crate::ship_view::ShipView;

// ── Pure visibility helper ────────────────────────────────────────────

/// Decide whether the navigation panel should be visible.
///
/// Rules:
/// 1. Game phase must be `InProgress`.
/// 2. The local player must hold `Console::Navigation`.
/// 3. If holding **one console only**, show automatically.
/// 4. If holding **multiple consoles**, show only when `ActiveConsole`
///    is explicitly set to `Navigation`.
pub fn navigation_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    let consoles = view.my_consoles();
    if !consoles.contains(&Console::Navigation) {
        return false;
    }
    let count = consoles.len();
    match &active.0 {
        Some(c) => *c == Console::Navigation,
        None => count == 1,
    }
}

// ── Pure impulse-status helper ────────────────────────────────────────

/// Derive the impulse status label from a charge progress value.
///
/// - `>= 1.0` → `"ACTIVE"`
/// - `> 0.0`  → `"Charging"`
/// - `<= 0.0` → `"Idle"`
pub fn impulse_status_label(charge: f32) -> &'static str {
    if charge >= 1.0 {
        "ACTIVE"
    } else if charge > 0.0 {
        "Charging"
    } else {
        "Idle"
    }
}

// ── State-visuals helpers ─────────────────────────────────────────────

/// Cancel-Impulse button: danger red.
fn cancel_impulse_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.40, 0.05, 0.05), // idle
        Color::srgb(0.55, 0.08, 0.08), // hover
        Color::srgb(0.60, 0.05, 0.05), // active
        Color::srgb(0.70, 0.12, 0.12), // press
        Color::srgb(0.15, 0.03, 0.03), // disabled
    )
}

/// On Screen button: neutral green.
fn on_screen_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.10, 0.30, 0.15), // idle
        Color::srgb(0.14, 0.42, 0.20), // hover
        Color::srgb(0.12, 0.50, 0.20), // active
        Color::srgb(0.18, 0.55, 0.25), // press
        Color::srgb(0.05, 0.15, 0.08), // disabled
    )
}

/// Impulse status text readout visuals.
fn impulse_readout_visuals() -> StateVisuals {
    StateVisuals::from_colors(
        Color::srgb(0.5, 1.0, 0.8), // idle
        Color::srgb(0.5, 1.0, 0.8), // hover (unused for readout)
        Color::srgb(0.2, 1.0, 0.5), // active (impulse active)
        Color::srgb(0.5, 1.0, 0.8), // press  (unused for readout)
        Color::srgb(0.3, 0.4, 0.4), // disabled
    )
}

// ── Marker components ────────────────────────────────────────────────

/// Marks the root of the Navigation console UI.
#[derive(Component)]
pub struct NavigationPanel;

/// Marks the navigation chart display container (gizmo-drawn).
#[derive(Component)]
pub struct NavChartPanel;

/// Marks the `TextReadout` root entity for the impulse status display.
#[derive(Component)]
pub struct NavImpulseReadout;

/// Marks the Cancel Impulse `GuiButton` entity so the refresh system can
/// show/hide it based on charge progress.
#[derive(Component)]
pub struct NavCancelImpulseButton;

// ── Plugin ────────────────────────────────────────────────────────────

/// Marker resource set once the navigation UI has been spawned.
#[derive(Resource)]
pub struct NavigationPanelSpawned;

pub struct NavigationPanelPlugin;

impl Plugin for NavigationPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                spawn_navigation_ui.run_if(not(resource_exists::<NavigationPanelSpawned>)),
                toggle_navigation_panel_visibility,
                refresh_navigation_panel,
                draw_nav_chart,
                respawn_navigation_on_orientation_change,
            ),
        );
    }
}

// ── Button observers ──────────────────────────────────────────────────

fn on_on_screen_pressed(
    _trigger: On<ButtonPressed>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    outbound.write(OutboundClientMessage(ClientMessage::SetView {
        mode: ViewMode::NavigationChart,
    }));
}

fn on_cancel_impulse_pressed(
    _trigger: On<ButtonPressed>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    outbound.write(OutboundClientMessage(ClientMessage::CancelImpulse));
}

// ── Spawn (ConsoleShell) ──────────────────────────────────────────────

fn spawn_navigation_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    old_panel: Query<Entity, With<NavigationPanel>>,
    old_help: Query<(Entity, &crate::client::elements::HelpOverlay)>,
    orientation: Option<Res<DeviceOrientation>>,
) {
    let Some(assets) = assets else { return };
    let is_landscape = crate::phone_border::framing::is_landscape(orientation.as_deref());

    for entity in old_panel.iter() {
        commands.entity(entity).despawn();
    }
    for (entity, overlay) in old_help.iter() {
        if overlay.0 == crate::client::elements::HelpPanel::Navigation {
            commands.entity(entity).despawn();
        }
    }

    commands.insert_resource(NavigationPanelSpawned);

    let shell = ConsoleShell::spawn(
        &mut commands,
        assets.helm_panel_bg.clone(),
        is_landscape,
        crate::client::elements::HelpPanel::Navigation,
        |commands: &mut Commands, primary: Entity| {
            fill_navigation_chart(commands, primary);
        },
        |commands: &mut Commands, secondary: Entity| {
            fill_navigation_controls(commands, secondary);
        },
        &assets,
    );

    commands.entity(shell.root).insert((NavigationPanel, Visibility::Hidden));
}

/// Primary slot: title + nav chart panel.
fn fill_navigation_chart(commands: &mut Commands, container: Entity) {
    let col = commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            row_gap: Val::Px(8.0),
            ..default()
        })
        .id();
    commands.entity(container).add_child(col);

    commands.entity(col).with_children(|p| {
        p.spawn((
            Text::new("Navigation"),
            TextFont { font_size: 32.0, ..default() },
            TextColor(Color::srgb(0.5, 1.0, 0.8)),
        ));
        p.spawn((
            NavChartPanel,
            Node {
                width: Val::Px(240.0),
                height: Val::Px(240.0),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BorderColor::all(Color::srgb(0.3, 1.0, 0.5)),
            BackgroundColor(Color::srgb(0.04, 0.06, 0.10)),
        ));
    });
}

/// Secondary slot: impulse status readout + cancel button + on-screen button.
fn fill_navigation_controls(commands: &mut Commands, container: Entity) {
    let col = commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            row_gap: Val::Px(12.0),
            ..default()
        })
        .id();
    commands.entity(container).add_child(col);

    let status_row = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(12.0),
            align_items: AlignItems::Center,
            ..default()
        })
        .id();
    commands.entity(col).add_child(status_row);

    let readout = TextReadout::spawn(commands, "Impulse:", impulse_readout_visuals());
    commands.entity(readout).insert(NavImpulseReadout);
    commands.entity(status_row).add_child(readout);

    let cancel_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect { width: 150.0, height: 36.0 },
        cancel_impulse_visuals(),
        None,
    );
    commands.entity(cancel_btn).insert((
        NavCancelImpulseButton,
        Visibility::Hidden,
    ));
    commands.entity(cancel_btn).with_children(|btn| {
        btn.spawn((
            Text::new("CANCEL IMPULSE"),
            TextFont { font_size: 14.0, ..default() },
            TextColor(Color::srgb(1.0, 0.4, 0.4)),
        ));
    });
    commands.entity(cancel_btn).observe(on_cancel_impulse_pressed);
    commands.entity(status_row).add_child(cancel_btn);

    let on_screen_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect { width: 120.0, height: 36.0 },
        on_screen_visuals(),
        None,
    );
    commands.entity(on_screen_btn).with_children(|btn| {
        btn.spawn((
            Text::new("ON SCREEN"),
            TextFont { font_size: 14.0, ..default() },
            TextColor(Color::srgb(0.5, 1.0, 0.5)),
        ));
    });
    commands.entity(on_screen_btn).observe(on_on_screen_pressed);
    commands.entity(col).add_child(on_screen_btn);
}

// ── Orientation respawn ──────────────────────────────────────────────

fn respawn_navigation_on_orientation_change(
    orientation: Option<Res<DeviceOrientation>>,
    panel: Query<Entity, With<NavigationPanel>>,
    mut commands: Commands,
) {
    let Some(orientation) = orientation else { return };
    if !orientation.is_changed() || orientation.is_added() {
        return;
    }
    for entity in panel.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<NavigationPanelSpawned>();
}

// ── Systems ──────────────────────────────────────────────────────────

fn toggle_navigation_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<NavigationPanel>>,
) {
    let visible = navigation_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

fn refresh_navigation_panel(
    ship_view: Res<ShipView>,
    mut readout: Query<&mut ReadoutValue, With<NavImpulseReadout>>,
    mut cancel_vis: Query<&mut Visibility, With<NavCancelImpulseButton>>,
) {
    if !ship_view.is_changed() {
        return;
    }
    let charge = ship_view.impulse_charge_progress;
    let label = impulse_status_label(charge);

    for mut value in readout.iter_mut() {
        value.0 = label.to_string();
    }
    for mut vis in cancel_vis.iter_mut() {
        *vis = if charge > 0.0 {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Draw the Navigation system chart on the `NavChartPanel` using gizmos.
///
/// Uses a star-centred, north-up projection so the star sits at the centre of
/// the display and north (+Y screen direction) always points toward world -Z.
/// The ship is shown as a heading triangle at its actual position relative to
/// the star (not locked to the chart centre).
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
    let Ok((node, gt, view_vis)) = panel.single() else {
        return;
    };
    if !view_vis.get() {
        return;
    }
    let Ok(window) = windows.single() else {
        return;
    };
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

    let config = crate::radar::navigation_chart_config();
    let chart_view = crate::radar::compute_star_centred_nav_chart(
        &sim.world.entities,
        ship_view.ship_x,
        ship_view.ship_z,
        &config,
    );
    const ZOOM: f32 = 1.0;

    for ring in &chart_view.rings {
        let pos = centre
            + Vec2::new(
                ring.centre_x * radius / ZOOM,
                ring.centre_y * radius / ZOOM,
            );
        let outer_r = ring.outer_r * radius / ZOOM;
        gizmos.circle_2d(pos, outer_r, Color::srgb(0.3, 0.7, 0.4));
        let inner_r = ring.inner_r * radius / ZOOM;
        if inner_r > 0.0 {
            gizmos.circle_2d(pos, inner_r, Color::srgb(0.2, 0.5, 0.3));
        }
    }

    let mut ship_radar_pos: Option<Vec2> = None;
    for dot in &chart_view.dots {
        let pos =
            centre + Vec2::new(dot.radar_x * radius / ZOOM, dot.radar_y * radius / ZOOM);
        if dot.uuid == crate::radar::SHIP_DOT_UUID {
            ship_radar_pos = Some(pos);
        } else {
            let pix_radius = (dot.scaled_radius * radius / ZOOM).max(3.0);
            gizmos.circle_2d(pos, pix_radius, Color::srgb(0.8, 0.8, 0.4));
        }
    }

    // Draw ship as a small heading triangle at its star-relative position.
    let ship_pos = ship_radar_pos.unwrap_or(centre);
    let yaw = ship_view.ship_yaw;
    let nose_len = radius * 0.10;
    let half_base = radius * 0.06;
    let (sin_y, cos_y) = yaw.sin_cos();
    let rotate =
        |v: Vec2| Vec2::new(v.x * cos_y + v.y * sin_y, -v.x * sin_y + v.y * cos_y);
    let nose = ship_pos + rotate(Vec2::new(0.0, nose_len));
    let left = ship_pos + rotate(Vec2::new(-half_base, -nose_len * 0.6));
    let right = ship_pos + rotate(Vec2::new(half_base, -nose_len * 0.6));
    gizmos.line_2d(nose, left, Color::srgb(0.5, 1.0, 0.8));
    gizmos.line_2d(left, right, Color::srgb(0.5, 1.0, 0.8));
    gizmos.line_2d(right, nose, Color::srgb(0.5, 1.0, 0.8));
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::messages::{Console, GamePhase, GameState, Player, ServerMessage, ShipClientConfig};
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

    fn in_progress_navigation_lobby(token: &str) -> LobbyState {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player(token, vec![Console::Navigation])],
        )));
        s
    }

    fn no_tab() -> ActiveConsole {
        ActiveConsole(None)
    }
    fn tab(c: Console) -> ActiveConsole {
        ActiveConsole(Some(c))
    }

    // ── navigation_panel_visible ──────────────────────────────────────

    #[test]
    fn navigation_panel_hidden_in_lobby_phase() {
        let lobby = LobbyState::default();
        let active = no_tab();
        assert!(!navigation_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn navigation_panel_hidden_when_player_not_navigation() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Helm])],
        )));
        let active = no_tab();
        assert!(!navigation_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn navigation_panel_visible_when_sole_console_and_no_tab() {
        let lobby = in_progress_navigation_lobby("tok");
        let active = no_tab();
        assert!(navigation_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn navigation_panel_visible_when_multi_console_and_navigation_tab() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Navigation, Console::Helm])],
        )));
        let active = tab(Console::Navigation);
        assert!(navigation_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn navigation_panel_hidden_when_multi_console_and_other_tab() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Navigation, Console::Helm])],
        )));
        let active = tab(Console::Helm);
        assert!(!navigation_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn navigation_panel_hidden_when_multi_console_and_no_tab() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Navigation, Console::Helm])],
        )));
        let active = no_tab();
        assert!(!navigation_panel_visible(&lobby, "tok", &active));
    }

    // ── impulse_status_label ──────────────────────────────────────────

    #[test]
    fn impulse_status_idle_when_zero_charge() {
        assert_eq!(impulse_status_label(0.0), "Idle");
    }

    #[test]
    fn impulse_status_charging_when_partial_charge() {
        assert_eq!(impulse_status_label(0.5), "Charging");
        assert_eq!(impulse_status_label(0.001), "Charging");
        assert_eq!(impulse_status_label(0.999), "Charging");
    }

    #[test]
    fn impulse_status_active_when_full_charge() {
        assert_eq!(impulse_status_label(1.0), "ACTIVE");
        assert_eq!(impulse_status_label(1.5), "ACTIVE");
    }

    #[test]
    fn impulse_status_idle_when_negative_charge() {
        assert_eq!(impulse_status_label(-0.1), "Idle");
    }

    // ── StateVisuals: five widget states ─────────────────────────────

    #[test]
    fn cancel_impulse_visuals_has_distinct_five_states() {
        use crate::gui::resolve_visual;
        let v = cancel_impulse_visuals();
        let idle = resolve_visual(&v, false, false, false, false).color;
        let hover = resolve_visual(&v, false, false, false, true).color;
        let active = resolve_visual(&v, false, false, true, false).color;
        let press = resolve_visual(&v, false, true, false, false).color;
        let disabled = resolve_visual(&v, true, false, false, false).color;
        assert_ne!(idle, hover);
        assert_ne!(idle, active);
        assert_ne!(idle, press);
        assert_ne!(idle, disabled);
    }

    #[test]
    fn on_screen_visuals_has_distinct_five_states() {
        use crate::gui::resolve_visual;
        let v = on_screen_visuals();
        let idle = resolve_visual(&v, false, false, false, false).color;
        let hover = resolve_visual(&v, false, false, false, true).color;
        let active = resolve_visual(&v, false, false, true, false).color;
        let press = resolve_visual(&v, false, true, false, false).color;
        let disabled = resolve_visual(&v, true, false, false, false).color;
        assert_ne!(idle, hover);
        assert_ne!(idle, active);
        assert_ne!(idle, press);
        assert_ne!(idle, disabled);
    }

    #[test]
    fn impulse_readout_visuals_idle_differs_from_active() {
        use crate::gui::resolve_visual;
        let v = impulse_readout_visuals();
        let idle = resolve_visual(&v, false, false, false, false).color;
        let active = resolve_visual(&v, false, false, true, false).color;
        assert_ne!(idle, active);
    }

    // ── ClientMessage variants (compile-time correctness checks) ─────

    #[test]
    fn cancel_impulse_message_variant_exists() {
        let msg = ClientMessage::CancelImpulse;
        assert_eq!(msg, ClientMessage::CancelImpulse);
    }

    #[test]
    fn set_view_navigation_chart_message_variant_exists() {
        let msg = ClientMessage::SetView {
            mode: ViewMode::NavigationChart,
        };
        if let ClientMessage::SetView { mode } = msg {
            assert_eq!(mode, ViewMode::NavigationChart);
        } else {
            panic!("expected SetView variant");
        }
    }
}
