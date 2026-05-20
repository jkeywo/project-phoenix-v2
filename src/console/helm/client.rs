//! Client-side Helm Panel plugin — migrated to `src/gui/` library widgets.
//!
//! Owns all helm console UI: joystick (`GenericJoystick`), radar (`GenericRadar`),
//! buttons (`GuiButton`), readouts (`TextReadout`), panel visibility, and 10 Hz
//! input resend. No per-button marker-component query systems remain.

use bevy::prelude::*;
use std::collections::HashMap;

use crate::client::console_shell::ConsoleShell;
use crate::client_app::{HelmPanel, OutboundClientMessage};
use crate::client_lobby::{ActiveConsole, LobbyState, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::gui::{
    ButtonPressed, ButtonSize, GenericJoystick, GenericRadar, JoystickMoved,
    OnRadar, OrientationMode, RadarAppearance, RadarCenter, RadarFilter,
    RadarLayer, RadarShape, ReadoutValue, StateVisuals, TextReadout, Visual,
    spawn_gui_button,
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

/// Fallback radar range in world units, used when the server has not yet
/// sent a `Welcome` with a `ShipClientConfig.helm_radar_range`. The real
/// runtime value comes from `LobbyState.ship_config.helm_radar_range`,
/// sourced from `[helm_console.radar] range` in `player_ship.toml`.
pub const HELM_RADAR_RANGE_FALLBACK: f32 = 500.0;

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
                respawn_helm_on_orientation_change,
                toggle_helm_panel_visibility,
                update_helm_readouts,
                sync_helm_radar_range,
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
/// No `is_changed()` gate — readouts must update every frame so that small
/// per-frame velocity / position deltas are reflected on the bezel without
/// waiting for a coarse change-detection tick.
fn update_helm_readouts(
    ship_view: Res<ShipView>,
    mut readouts: Query<(&mut ReadoutValue, &HelmReadoutKind)>,
) {
    for (mut value, kind) in readouts.iter_mut() {
        *value = match kind {
            HelmReadoutKind::Hdg => ReadoutValue(format!("HDG {}", yaw_to_heading(ship_view.ship_yaw))),
            HelmReadoutKind::Spd => ReadoutValue(format!("SPD {:.0}", ship_view.forward_speed)),
            HelmReadoutKind::X => ReadoutValue(format!("X {:.0}", ship_view.ship_x)),
            HelmReadoutKind::Z => ReadoutValue(format!("Z {:.0}", ship_view.ship_z)),
        };
    }
}

/// Keeps `GenericRadarWidget.range` in sync with the server-provided
/// `LobbyState.ship_config.helm_radar_range`. Runs whenever `LobbyState`
/// changes (i.e. on `Welcome`) so the helm radar uses the TOML-driven
/// range from `[helm_console.radar]`.
fn sync_helm_radar_range(
    lobby: Res<LobbyState>,
    mut radars: Query<&mut crate::gui::GenericRadarWidget>,
) {
    if !lobby.is_changed() {
        return;
    }
    let range = lobby.ship_config.helm_radar_range;
    if range <= 0.0 {
        return;
    }
    for mut widget in radars.iter_mut() {
        widget.range = range;
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
                GlobalTransform::default(),
            )).id();
            radar.blips.insert(uuid.clone(), blip);
        }
    }

    // Despawn blips that are no longer in the sim state
    radar.blips.retain(|uuid, entity| {
        if seen.contains(uuid) {
            true
        } else {
            commands.entity(*entity).despawn_related::<Children>();
            false
        }
    });
}

// ── Phone helm UI spawn ──────────────────────────────────────────────

/// Marker resource set once the phone helm UI has been spawned.
#[derive(Resource)]
pub struct PhoneHelmSpawned;

/// Spawns the phone helm panel using `ConsoleShell::spawn` (PRD #346).
fn spawn_phone_helm_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    old_panel: Query<Entity, With<HelmPanel>>,
    orientation: Option<Res<DeviceOrientation>>,
) {
    let Some(assets) = assets else { return };
    let is_landscape = crate::phone_border::framing::is_landscape(orientation.as_deref());

    for entity in old_panel.iter() {
        commands.entity(entity).despawn_related::<Children>();
    }

    commands.insert_resource(PhoneHelmSpawned);

    let shell = ConsoleShell::spawn(
        &mut commands,
        assets.helm_panel_bg.clone(),
        is_landscape,
        |commands: &mut Commands, primary: Entity| {
            fill_helm_radar(commands, primary, &assets);
        },
        |commands: &mut Commands, secondary: Entity| {
            fill_helm_joystick(commands, secondary, &assets);
        },
        &assets,
    );

    // Insert HelmPanel marker on the root for visibility queries
    commands.entity(shell.root).insert(HelmPanel);
}

// ── Fill helpers ─────────────────────────────────────────────────────

fn fill_helm_radar(commands: &mut Commands, container: Entity, assets: &PhoneAssets) {
    let dim = Color::srgb(0.55, 0.70, 1.0);
    let readout_visuals = || StateVisuals::from_colors(
        dim, dim, dim, dim, Color::srgb(0.3, 0.4, 0.6),
    );
    let display = |s: f32| TextFont { font: assets.font_display.clone(), font_size: s, ..default() };

    // ── Column wrapper to stack radar + buttons vertically ──
    let col = commands.spawn(Node {
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        row_gap: Val::Px(8.0),
        ..default()
    }).id();
    commands.entity(container).add_child(col);

    // ── Radar ──
    let radar_filter = RadarFilter(std::collections::HashSet::from([
        RadarLayer::Ship, RadarLayer::Asteroid,
        RadarLayer::Station, RadarLayer::Missile,
        RadarLayer::Planet, RadarLayer::Star,
    ]));
    let radar = GenericRadar::spawn(
        commands, COMPASS_RADAR_DIAMETER, HELM_RADAR_RANGE_FALLBACK,
        OrientationMode::ShipRelative, radar_filter,
        Some(assets.radar_bg.clone()), Some(assets.radar_surround.clone()),
    );
    commands.entity(col).add_child(radar);

    // HDG readout
    let hdg = TextReadout::spawn(commands, "HDG", readout_visuals());
    commands.entity(hdg).insert((HelmReadoutKind::Hdg, Node {
        position_type: PositionType::Absolute,
        left: Val::Px(2.0), top: Val::Px(2.0), ..default()
    }));
    commands.entity(radar).add_child(hdg);

    // SPD readout
    let spd = TextReadout::spawn(commands, "SPD", readout_visuals());
    commands.entity(spd).insert((HelmReadoutKind::Spd, Node {
        position_type: PositionType::Absolute,
        right: Val::Px(2.0), top: Val::Px(2.0), ..default()
    }));
    commands.entity(radar).add_child(spd);

    // X readout
    let x_read = TextReadout::spawn(commands, "X", readout_visuals());
    commands.entity(x_read).insert((HelmReadoutKind::X, Node {
        position_type: PositionType::Absolute,
        left: Val::Px(2.0), bottom: Val::Px(2.0), ..default()
    }));
    commands.entity(radar).add_child(x_read);

    // Z readout
    let z_read = TextReadout::spawn(commands, "Z", readout_visuals());
    commands.entity(z_read).insert((HelmReadoutKind::Z, Node {
        position_type: PositionType::Absolute,
        right: Val::Px(2.0), bottom: Val::Px(2.0), ..default()
    }));
    commands.entity(radar).add_child(z_read);

    // ── Buttons row ──
    let buttons_row = commands.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        column_gap: Val::Px(8.0),
        ..default()
    }).id();

    // ON SCREEN button with button-normal PNG visuals
    let on_screen_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect { width: 120.0, height: 32.0 },
        StateVisuals {
            idle: Visual { image: Some(assets.btn_normal_idle.clone()), color: Color::NONE },
            hover: Visual { image: Some(assets.btn_normal_hover.clone()), color: Color::NONE },
            active: Visual { image: Some(assets.btn_normal_active.clone()), color: Color::NONE },
            press: Visual { image: Some(assets.btn_normal_press.clone()), color: Color::NONE },
            disabled: Visual { image: None, color: Color::srgba(0.08, 0.08, 0.15, 0.5) },
        },
        None,
    );
    commands.entity(on_screen_btn).with_children(|btn| {
        btn.spawn((Text::new("ON SCREEN"), display(12.0), TextColor(Color::srgb(0.93, 0.93, 1.0))));
    });
    commands.entity(on_screen_btn).observe(on_on_screen_button_pressed);
    commands.entity(buttons_row).add_child(on_screen_btn);

    // IMPULSE button with impulse PNG visuals
    let impulse_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect { width: 120.0, height: 32.0 },
        StateVisuals {
            idle: Visual { image: Some(assets.impulse_ready.clone()), color: Color::NONE },
            hover: Visual { image: Some(assets.impulse_hover.clone()), color: Color::NONE },
            active: Visual { image: Some(assets.impulse_active.clone()), color: Color::NONE },
            press: Visual { image: Some(assets.impulse_press.clone()), color: Color::NONE },
            disabled: Visual { image: None, color: Color::srgba(0.05, 0.10, 0.20, 0.5) },
        },
        None,
    );
    commands.entity(impulse_btn).with_children(|btn| {
        btn.spawn((Text::new("IMPULSE"), display(12.0), TextColor(Color::srgb(0.5, 0.8, 1.0))));
    });
    commands.entity(impulse_btn).observe(on_impulse_button_pressed);
    commands.entity(buttons_row).add_child(impulse_btn);

    commands.entity(col).add_child(buttons_row);
}

fn fill_helm_joystick(commands: &mut Commands, container: Entity, assets: &PhoneAssets) {
    let dim = Color::srgb(0.55, 0.70, 1.0);
    let muted = Color::srgb(0.6, 0.7, 0.73);
    let mono = |s: f32| TextFont { font: assets.font_mono.clone(), font_size: s, ..default() };

    // ── Column wrapper to stack joystick elements vertically ──
    let col = commands.spawn(Node {
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        row_gap: Val::Px(4.0),
        ..default()
    }).id();
    commands.entity(container).add_child(col);

    // Directional indicators
    let fwd = commands.spawn((Text::new("▲"), mono(16.0), TextColor(dim), Node::default())).id();
    commands.entity(col).add_child(fwd);

    let joystick_center_row = commands.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(4.0),
        ..default()
    }).id();
    commands.entity(col).add_child(joystick_center_row);

    // Left directional chevron
    let left_arrow = commands.spawn((Text::new("◄"), mono(14.0), TextColor(dim), Node::default())).id();
    commands.entity(joystick_center_row).add_child(left_arrow);

    // Joystick with PNG visuals
    let joystick_knob_visuals = StateVisuals {
        idle: Visual { image: Some(assets.joystick_knob_idle.clone()), color: Color::NONE },
        hover: Visual { image: Some(assets.joystick_knob_hover.clone()), color: Color::NONE },
        active: Visual { image: Some(assets.joystick_knob_active.clone()), color: Color::NONE },
        press: Visual { image: Some(assets.joystick_knob_press.clone()), color: Color::NONE },
        disabled: Visual { image: None, color: Color::srgba(0.05, 0.05, 0.10, 0.5) },
    };
    let pad = GenericJoystick::spawn(
        commands, HELM_PAD_SIZE,
        Some(assets.joystick_pad_idle.clone()),
        Some(assets.joystick_knob_idle.clone()),
        joystick_knob_visuals,
    );
    commands.entity(pad).observe(on_helm_joystick_moved);
    commands.entity(joystick_center_row).add_child(pad);

    // Right directional chevron
    let right_arrow = commands.spawn((Text::new("►"), mono(14.0), TextColor(dim), Node::default())).id();
    commands.entity(joystick_center_row).add_child(right_arrow);

    let aft = commands.spawn((Text::new("▼"), mono(16.0), TextColor(dim), Node::default())).id();
    commands.entity(col).add_child(aft);

    // FWD/REV labels
    let fwd_rev_row = commands.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        column_gap: Val::Px(24.0),
        ..default()
    }).id();
    commands.entity(fwd_rev_row).with_children(|row| {
        row.spawn((Text::new("FWD"), mono(9.0), TextColor(muted)));
        row.spawn((Text::new("REV"), mono(9.0), TextColor(muted)));
    });
    commands.entity(col).add_child(fwd_rev_row);

    // PORT/STBD labels
    let port_stbd_row = commands.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        column_gap: Val::Px(32.0),
        ..default()
    }).id();
    commands.entity(port_stbd_row).with_children(|row| {
        row.spawn((Text::new("PORT"), mono(9.0), TextColor(muted)));
        row.spawn((Text::new("STBD"), mono(9.0), TextColor(muted)));
    });
    commands.entity(col).add_child(port_stbd_row);
}

// ── Orientation respawn ──────────────────────────────────────────────

/// When `DeviceOrientation` changes, despawn the current `HelmPanel` and
/// remove the `PhoneHelmSpawned` resource so `spawn_phone_helm_ui` respawns
/// with the correct layout.
fn respawn_helm_on_orientation_change(
    orientation: Res<DeviceOrientation>,
    panel: Query<Entity, With<HelmPanel>>,
    mut commands: Commands,
    mut radar: ResMut<HelmRadarEntities>,
) {
    if !orientation.is_changed() {
        return;
    }
    for entity in panel.iter() {
        commands.entity(entity).despawn_related::<Children>();
    }
    // Clear stale entity IDs so bridge systems don't command dead entities.
    radar.center = None;
    radar.blips.clear();
    commands.remove_resource::<PhoneHelmSpawned>();
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::messages::{Console, GamePhase, GameState, Player, ServerMessage, ShipClientConfig};
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
        ServerMessage::Welcome { state, ship_stations: ShipStations::default(), ship_config: ShipClientConfig::default() }
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
