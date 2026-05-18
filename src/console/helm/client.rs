//! Client-side Helm Panel plugin — migrated to `src/gui/` library widgets.
//!
//! Owns all helm console UI: joystick (`GenericJoystick`), radar (`GenericRadar`),
//! buttons (`GuiButton`), readouts (`TextReadout`), panel visibility, and 10 Hz
//! input resend. No per-button marker-component query systems remain.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::client_app::{HelmPanel, OutboundClientMessage};
use crate::client_lobby::{ActiveConsole, LobbyState, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::gui::{
    ButtonPressed, GenericJoystick, JoystickMoved, OnRadar, OrientationMode,
    RadarAppearance, RadarCenter, RadarFilter, RadarLayer, RadarShape,
    ReadoutValue, TextReadout,
};
use crate::messages::{ClientMessage, Console, GamePhase, ViewMode};
use crate::phone_border::framing::{DeviceOrientation, PhoneAssets};
use crate::ship_view::ShipView;

// ── Pure helpers ─────────────────────────────────────────────────────

/// Decide whether the helm panel should be visible.
pub fn helm_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    use crate::client_lobby::LobbyView;
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    if !view.is_helm() {
        return false;
    }
    let count = view.my_consoles().len();
    match &active.0 {
        Some(c) => *c == Console::Helm,
        None => count == 1,
    }
}

// ── Resources ────────────────────────────────────────────────────────

/// 10 Hz resend timer for the helm joystick.
///
/// The `GenericJoystick` has its own 10 Hz resend via `JoystickResendTimer`
/// but the helm plugin keeps this resource for backward-compat systems
/// that may reference it (e.g. `toggle_helm_panel_visibility`).
#[derive(Resource)]
pub struct HelmTickTimer(pub Timer);

/// Persistent entity IDs for helm-specific radar components.
#[derive(Resource, Default)]
struct HelmRadarEntities {
    center: Option<Entity>,
    blips: HashMap<String, Entity>,
}

// ── Readout kind discriminator ───────────────────────────────────────

/// Distinguishes which helm readout a `TextReadout` root belongs to.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
enum HelmReadoutKind {
    Hdg,
    Spd,
    X,
    Z,
}

// ── Constants ────────────────────────────────────────────────────────

/// Diameter of the joystick pad in logical pixels.
pub const HELM_PAD_SIZE: f32 = 200.0;

/// Diameter of the compass-ring radar in logical pixels.
pub const COMPASS_RADAR_DIAMETER: f32 = 280.0;

/// Radar range in world units for the helm generic radar.
pub const HELM_RADAR_RANGE: f32 = 500.0;

/// Convert ship yaw (radians, CCW from +Z) to a 3-digit heading string
/// (degrees, 0–360, 0 = ship-forward = "north" on the compass).
pub fn yaw_to_heading(yaw_rad: f32) -> String {
    let heading = ((-yaw_rad).to_degrees()).rem_euclid(360.0);
    format!("{:03}°", heading.round() as u32)
}

// ── Plugin ───────────────────────────────────────────────────────────

pub struct HelmPanelPlugin;

impl Plugin for HelmPanelPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HelmTickTimer(Timer::from_seconds(0.1, TimerMode::Repeating)))
            .init_resource::<HelmRadarEntities>()
            .add_systems(Update, (
                spawn_phone_helm_ui
                    .run_if(not(resource_exists::<PhoneHelmSpawned>)),
                toggle_helm_panel_visibility,
                update_helm_readouts,
                bridge_client_sim_to_radar_entities,
            ));
    }
}

// ── Visibility system ────────────────────────────────────────────────

fn toggle_helm_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<HelmPanel>>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    let visible = helm_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
    if !visible {
        // Send zero input when panel is hidden
        outbound.write(OutboundClientMessage(
            ClientMessage::HelmInput { thrust: 0.0, steering: 0.0 },
        ));
    }
}

// ── Joystick observer ────────────────────────────────────────────────

fn on_helm_joystick_moved(
    trigger: On<JoystickMoved>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    let moved = trigger.event();
    let thrust = -moved.dy;
    let steering = moved.dx;
    outbound.write(OutboundClientMessage(
        ClientMessage::HelmInput { thrust, steering },
    ));
}

// ── Button observers ─────────────────────────────────────────────────

fn on_on_screen_button_pressed(
    _trigger: On<ButtonPressed>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    outbound.write(OutboundClientMessage(ClientMessage::SetView { mode: ViewMode::Radar }));
}

fn on_impulse_button_pressed(
    _trigger: On<ButtonPressed>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    outbound.write(OutboundClientMessage(ClientMessage::StartImpulseCharge));
}

// ── Readout update system ────────────────────────────────────────────

/// Each frame: write HDG / SPD / X / Z readout values from `ShipView`.
fn update_helm_readouts(
    ship_view: Res<ShipView>,
    mut readouts: Query<(&mut ReadoutValue, &HelmReadoutKind)>,
) {
    if !ship_view.is_changed() {
        return;
    }
    for (mut value, kind) in readouts.iter_mut() {
        *value = match kind {
            HelmReadoutKind::Hdg => ReadoutValue(format!("HDG {}", yaw_to_heading(ship_view.ship_yaw))),
            HelmReadoutKind::Spd => ReadoutValue(format!("SPD {:.0}", ship_view.forward_speed)),
            HelmReadoutKind::X => ReadoutValue(format!("X {:.0}", ship_view.ship_x)),
            HelmReadoutKind::Z => ReadoutValue(format!("Z {:.0}", ship_view.ship_z)),
        };
    }
}

// ── Radar entity bridge ─────────────────────────────────────────────

/// Bridges `ClientSimState` entity snapshots into ECS entities with
/// `OnRadar` / `RadarAppearance` for the `GenericRadar` widget.
fn bridge_client_sim_to_radar_entities(
    mut commands: Commands,
    sim: Res<ClientSimState>,
    ship_view: Res<ShipView>,
    mut radar: ResMut<HelmRadarEntities>,
) {
    // Manage the RadarCenter entity
    let _center_entity = match radar.center {
        Some(e) => {
            commands.entity(e).insert(RadarCenter {
                world_x: ship_view.ship_x,
                world_z: ship_view.ship_z,
                yaw: ship_view.ship_yaw,
            });
            e
        }
        None => {
            let e = commands.spawn(RadarCenter {
                world_x: ship_view.ship_x,
                world_z: ship_view.ship_z,
                yaw: ship_view.ship_yaw,
            }).id();
            radar.center = Some(e);
            e
        }
    };

    // De-duplicate: keep track of which UUIDs we see this frame
    let mut seen = std::collections::HashSet::new();

    for snapshot in &sim.world.entities {
        let uuid = &snapshot.uuid;
        if !seen.insert(uuid.clone()) {
            continue;
        }

        let has_tag = |tag: &str| snapshot.tags.iter().any(|t| t == tag);
        if has_tag("region") {
            continue; // skip regions — not radar-relevant
        }
        let layer = if has_tag("ship") || has_tag("pirate") {
            RadarLayer::Ship
        } else if has_tag("asteroid") || has_tag("asteroid_field") {
            RadarLayer::Asteroid
        } else if has_tag("station") {
            RadarLayer::Station
        } else if has_tag("missile") || has_tag("torpedo") {
            RadarLayer::Missile
        } else if has_tag("planet") {
            RadarLayer::Planet
        } else if has_tag("star") {
            RadarLayer::Star
        } else {
            continue; // unknown type
        };

        let colour = snapshot.colour.map(|c| Color::srgb(c[0], c[1], c[2]));
        let default_color = match layer {
            RadarLayer::Ship => Color::srgb(0.95, 0.95, 1.0),
            RadarLayer::Asteroid => Color::srgb(0.85, 0.75, 0.45),
            RadarLayer::Station => Color::srgb(0.3, 0.8, 0.6),
            RadarLayer::Missile => Color::srgb(1.0, 0.4, 0.2),
            RadarLayer::Planet => Color::srgb(0.0, 0.6, 1.0),
            RadarLayer::Star => Color::srgb(1.0, 0.85, 0.3),
        };
        let shape = if has_tag("ship") {
            RadarShape::Triangle
        } else if has_tag("station") {
            RadarShape::Square
        } else {
            RadarShape::Dot
        };
        let appearance = RadarAppearance {
            color: colour.unwrap_or(default_color),
            radius: snapshot.radius_or_zero().max(2.0),
            shape,
        };

        if let Some(existing) = radar.blips.get(uuid) {
            commands.entity(*existing).insert((
                OnRadar(layer),
                appearance,
                Transform::from_xyz(snapshot.x(), 0.0, snapshot.z()),
            ));
        } else {
            let blip = commands.spawn((
                OnRadar(layer),
                appearance,
                Transform::from_xyz(snapshot.x(), 0.0, snapshot.z()),
            )).id();
            radar.blips.insert(uuid.clone(), blip);
        }
    }

    // Despawn blips that are no longer in the sim state
    radar.blips.retain(|uuid, entity| {
        if seen.contains(uuid) {
            true
        } else {
            commands.entity(*entity).despawn();
            false
        }
    });
}

// ── Phone helm UI spawn ──────────────────────────────────────────────

/// Marker resource set once the phone helm UI has been spawned.
#[derive(Resource)]
pub struct PhoneHelmSpawned;

/// Spawns the phone helm panel using gui library widgets.
fn spawn_phone_helm_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    old_panel: Query<Entity, With<HelmPanel>>,
    orientation: Option<Res<DeviceOrientation>>,
) {
    let Some(assets) = assets else { return };
    let is_landscape = matches!(orientation.as_deref(), Some(DeviceOrientation::Landscape));

    for entity in old_panel.iter() {
        commands.entity(entity).despawn();
    }

    commands.insert_resource(PhoneHelmSpawned);

    let mono = |s: f32| TextFont { font: assets.font_mono.clone(), font_size: s, ..default() };
    let display = |s: f32| TextFont { font: assets.font_display.clone(), font_size: s, ..default() };
    let dim = Color::srgb(0.55, 0.70, 1.0);
    let muted = Color::srgb(0.6, 0.7, 0.73);
    let readout_visuals = || crate::gui::StateVisuals::from_colors(
        dim, dim, dim, dim, Color::srgb(0.3, 0.4, 0.6),
    );

    // ── Pre-spawn widget entities (outside any with_children closures) ──

    let joystick_visuals = crate::gui::StateVisuals::from_colors(
        Color::srgb(0.10, 0.10, 0.18), Color::srgb(0.10, 0.10, 0.18),
        Color::srgb(0.10, 0.10, 0.18), Color::srgb(0.10, 0.10, 0.18),
        Color::srgb(0.05, 0.05, 0.10),
    );
    let joystick_pad = GenericJoystick::spawn(
        &mut commands, HELM_PAD_SIZE, None, None, joystick_visuals,
    );
    commands.entity(joystick_pad).observe(on_helm_joystick_moved);

    let radar_filter = RadarFilter(std::collections::HashSet::from([
        RadarLayer::Ship, RadarLayer::Asteroid,
        RadarLayer::Station, RadarLayer::Missile,
        RadarLayer::Planet, RadarLayer::Star,
    ]));
    let radar = crate::gui::GenericRadar::spawn(
        &mut commands, COMPASS_RADAR_DIAMETER, HELM_RADAR_RANGE,
        OrientationMode::ShipRelative, radar_filter,
        Some(assets.compass_ring.clone()), None,
    );

    let hdg = TextReadout::spawn(&mut commands, "HDG", readout_visuals());
    commands.entity(hdg).insert((HelmReadoutKind::Hdg, Node {
        position_type: PositionType::Absolute,
        left: Val::Px(2.0), top: Val::Px(2.0), ..default()
    }));
    commands.entity(radar).add_child(hdg);

    let spd = TextReadout::spawn(&mut commands, "SPD", readout_visuals());
    commands.entity(spd).insert((HelmReadoutKind::Spd, Node {
        position_type: PositionType::Absolute,
        right: Val::Px(2.0), top: Val::Px(2.0), ..default()
    }));
    commands.entity(radar).add_child(spd);

    let x_read = TextReadout::spawn(&mut commands, "X", readout_visuals());
    commands.entity(x_read).insert((HelmReadoutKind::X, Node {
        position_type: PositionType::Absolute,
        left: Val::Px(2.0), bottom: Val::Px(2.0), ..default()
    }));
    commands.entity(radar).add_child(x_read);

    let z_read = TextReadout::spawn(&mut commands, "Z", readout_visuals());
    commands.entity(z_read).insert((HelmReadoutKind::Z, Node {
        position_type: PositionType::Absolute,
        right: Val::Px(2.0), bottom: Val::Px(2.0), ..default()
    }));
    commands.entity(radar).add_child(z_read);

    let on_screen_btn = crate::gui::spawn_gui_button(
        &mut commands,
        crate::gui::ButtonSize::Rect { width: 120.0, height: 32.0 },
        crate::gui::StateVisuals::from_colors(
            Color::srgb(0.13, 0.13, 0.27), Color::srgb(0.13, 0.13, 0.27),
            Color::srgb(0.10, 0.40, 0.15), Color::srgb(0.13, 0.13, 0.27),
            Color::srgb(0.08, 0.08, 0.15),
        ),
        None,
    );
    commands.entity(on_screen_btn).with_children(|btn| {
        btn.spawn((Text::new("ON SCREEN"), display(12.0), TextColor(Color::srgb(0.93, 0.93, 1.0))));
    });
    commands.entity(on_screen_btn).observe(on_on_screen_button_pressed);

    let impulse_btn = crate::gui::spawn_gui_button(
        &mut commands,
        crate::gui::ButtonSize::Rect { width: 120.0, height: 32.0 },
        crate::gui::StateVisuals::from_colors(
            Color::srgb(0.10, 0.25, 0.40), Color::srgb(0.10, 0.25, 0.40),
            Color::srgb(0.15, 0.35, 0.50), Color::srgb(0.10, 0.25, 0.40),
            Color::srgb(0.05, 0.10, 0.20),
        ),
        None,
    );
    commands.entity(impulse_btn).with_children(|btn| {
        btn.spawn((Text::new("IMPULSE"), display(12.0), TextColor(Color::srgb(0.5, 0.8, 1.0))));
    });
    commands.entity(impulse_btn).observe(on_impulse_button_pressed);

    // ── Spawn panel and build flat hierarchy using add_child ──

    let panel = commands.spawn((
        HelmPanel,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(4.0), right: Val::Px(4.0),
            top: Val::Px(4.0), bottom: Val::Px(4.0),
            flex_direction: FlexDirection::Column,
            ..default()
        },
        Visibility::Hidden,
    )).id();

    // Title bar
    let title_row = commands.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        ..default()
    }).id();
    commands.entity(title_row).with_children(|tr| {
        tr.spawn((Text::new("Helm"), TextFont { font_size: 18.0, ..default() }, TextColor(Color::srgb(0.3, 1.0, 0.8))));
        crate::client_elements::spawn_help_button(tr, crate::client_elements::HelpPanel::Helm, 14.0);
    });
    commands.entity(panel).add_child(title_row);

    commands.entity(panel).with_children(|root| {
        crate::client_elements::spawn_help_overlay(root, crate::client_elements::HelpPanel::Helm);
    });

    // Content row
    let content_row = commands.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::FlexEnd,
        justify_content: JustifyContent::SpaceBetween,
        flex_grow: 1.0,
        column_gap: Val::Px(8.0),
        ..default()
    }).id();
    commands.entity(panel).add_child(content_row);

    // ── Build joystick column (all entities spawned flat) ──
    let joystick_col = commands.spawn(Node {
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        row_gap: Val::Px(4.0),
        ..default()
    }).id();

    let fwd_indicator = commands.spawn((Text::new("▲"), mono(16.0), TextColor(dim), Node::default())).id();
    commands.entity(joystick_col).add_child(fwd_indicator);

    let joystick_center_row = commands.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(4.0),
        ..default()
    }).id();
    let left_indicator = commands.spawn((Text::new("◄"), mono(14.0), TextColor(dim), Node::default())).id();
    let right_indicator = commands.spawn((Text::new("►"), mono(14.0), TextColor(dim), Node::default())).id();
    commands.entity(joystick_center_row).add_child(left_indicator);
    commands.entity(joystick_center_row).add_child(joystick_pad);
    commands.entity(joystick_center_row).add_child(right_indicator);
    commands.entity(joystick_col).add_child(joystick_center_row);

    let aft_indicator = commands.spawn((Text::new("▼"), mono(16.0), TextColor(dim), Node::default())).id();
    commands.entity(joystick_col).add_child(aft_indicator);

    let fwd_rev_row = commands.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        column_gap: Val::Px(24.0),
        ..default()
    }).id();
    let fwd_label = commands.spawn((Text::new("FWD"), mono(9.0), TextColor(muted))).id();
    let rev_label = commands.spawn((Text::new("REV"), mono(9.0), TextColor(muted))).id();
    commands.entity(fwd_rev_row).add_child(fwd_label);
    commands.entity(fwd_rev_row).add_child(rev_label);
    commands.entity(joystick_col).add_child(fwd_rev_row);

    let port_stbd_row = commands.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        column_gap: Val::Px(32.0),
        ..default()
    }).id();
    let port_label = commands.spawn((Text::new("PORT"), mono(9.0), TextColor(muted))).id();
    let stbd_label = commands.spawn((Text::new("STBD"), mono(9.0), TextColor(muted))).id();
    commands.entity(port_stbd_row).add_child(port_label);
    commands.entity(port_stbd_row).add_child(stbd_label);
    commands.entity(joystick_col).add_child(port_stbd_row);

    // ── Build radar column ──
    let radar_col = commands.spawn(Node {
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        row_gap: Val::Px(6.0),
        ..default()
    }).id();
    commands.entity(radar_col).add_child(radar);

    let buttons_row = commands.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        column_gap: Val::Px(8.0),
        ..default()
    }).id();
    commands.entity(buttons_row).add_child(on_screen_btn);
    commands.entity(buttons_row).add_child(impulse_btn);
    commands.entity(radar_col).add_child(buttons_row);

    // ── Attach columns to content row ──
    if is_landscape {
        commands.entity(content_row).add_child(radar_col);
        commands.entity(content_row).add_child(joystick_col);
    } else {
        commands.entity(content_row).add_child(joystick_col);
        commands.entity(content_row).add_child(radar_col);
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::messages::{Console, GamePhase, GameState, Player, ServerMessage};
    use crate::stations_config::ShipStations;
    use std::collections::HashMap;
    use std::f32::consts::{FRAC_PI_4, TAU};

    // ── helm_panel_visible ────────────────────────────────────────────

    fn player(token: &str, consoles: Vec<Console>) -> Player {
        Player { token: token.into(), name: "test".into(), consoles, connected: true }
    }

    fn game_state(phase: GamePhase, players: Vec<Player>) -> GameState {
        GameState { phase, players, complexity: HashMap::new(), world: None }
    }

    fn welcome(state: GameState) -> ServerMessage {
        ServerMessage::Welcome { state, ship_stations: ShipStations::default() }
    }

    fn in_progress_helm_lobby(token: &str) -> LobbyState {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player(token, vec![Console::Helm])],
        )));
        s
    }

    fn no_tab() -> ActiveConsole { ActiveConsole(None) }
    fn tab(c: Console) -> ActiveConsole { ActiveConsole(Some(c)) }

    #[test]
    fn helm_panel_hidden_in_lobby_phase() {
        let lobby = LobbyState::default();
        let active = no_tab();
        assert!(!helm_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn helm_panel_hidden_when_player_not_helm() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::CaptainChair])],
        )));
        let active = no_tab();
        assert!(!helm_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn helm_panel_visible_when_sole_console_and_no_tab() {
        let lobby = in_progress_helm_lobby("tok");
        let active = no_tab();
        assert!(helm_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn helm_panel_visible_when_multi_console_and_helm_tab() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Helm, Console::Tactical])],
        )));
        let active = tab(Console::Helm);
        assert!(helm_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn helm_panel_hidden_when_multi_console_and_other_tab() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Helm, Console::Tactical])],
        )));
        let active = tab(Console::Tactical);
        assert!(!helm_panel_visible(&lobby, "tok", &active));
    }

    #[test]
    fn helm_panel_hidden_when_multi_console_and_no_tab() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("tok", vec![Console::Helm, Console::Tactical])],
        )));
        let active = no_tab();
        assert!(!helm_panel_visible(&lobby, "tok", &active));
    }

    // ── yaw_to_heading ─────────────────────────────────────────────

    #[test]
    fn yaw_zero_is_heading_000() {
        assert_eq!(yaw_to_heading(0.0), "000°");
    }

    #[test]
    fn yaw_negative_quarter_turn_is_heading_045() {
        assert_eq!(yaw_to_heading(-FRAC_PI_4), "045°");
    }

    #[test]
    fn yaw_pi_is_heading_180() {
        assert_eq!(yaw_to_heading(std::f32::consts::PI), "180°");
    }

    #[test]
    fn yaw_negative_yaw_wraps_correctly() {
        let h = yaw_to_heading(-0.5);
        assert_eq!(h, "029°");
    }

    #[test]
    fn yaw_2pi_wraps_to_000() {
        assert_eq!(yaw_to_heading(TAU), "000°");
    }

    #[test]
    fn yaw_negative_angle_always_positive_heading() {
        let h = yaw_to_heading(-TAU);
        assert_eq!(h, "000°");
        let h2 = yaw_to_heading(-0.1);
        assert!(!h2.starts_with('-'));
    }
}
