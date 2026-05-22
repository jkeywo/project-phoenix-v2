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
    default_layer_colour, layer_to_icon, region_shape_from_snapshot, spawn_gui_button,
    tags_to_radar_layer, ButtonPressed, ButtonSize, GenericJoystick, GenericRadar,
    GenericRadarWidget, HelmRadarWidget, JoystickMoved, OnRadar, OrientationMode, RadarAppearance,
    RadarCenter, RadarClipMode, RadarFilter, RadarIcon, RadarLayer, ReadoutValue, StateVisuals,
    TextReadout, Visual,
};
use crate::messages::{ClientMessage, Console, GamePhase, ViewMode};
use crate::phone_border::framing::{DeviceOrientation, PhoneAssets};
use crate::ship_view::ShipView;
use crate::console::helm::joystick::{
    format_impulse_status, impulse_ui_visibility, should_send_helm_input,
};
use crate::gui::{
    reset_joystick_drag, JoystickDragState, JoystickResendTimer,
};

// ── Pure helpers ─────────────────────────────────────────────────────

/// Decide whether the helm panel should be visible.
pub fn helm_panel_visible(lobby: &LobbyState, token: &str, active: &ActiveConsole) -> bool {
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

/// Fallback radar range in world units, used when the server has not yet
/// sent a `Welcome` with a `ShipClientConfig.helm_radar_range`. The real
/// runtime value comes from `LobbyState.ship_config.helm_radar_range`,
/// sourced from `[helm_console.radar] range` in `player_ship.toml`.
pub const HELM_RADAR_RANGE_FALLBACK: f32 = 250.0;

/// Convert ship yaw (radians, CCW from +Z) to a 3-digit heading string
/// (degrees, 0–360, 0 = ship-forward = "north" on the compass).
pub fn yaw_to_heading(yaw_rad: f32) -> String {
    let heading = ((-yaw_rad).to_degrees()).rem_euclid(360.0);
    format!("{:03}°", heading.round() as u32)
}

// ── Impulse UI markers ───────────────────────────────────────────────

/// Marks the joystick pad so we can hide it while the impulse drive is
/// charging or active (autopilot mode).
#[derive(Component)]
struct HelmJoystickPad;

/// Marks the inner `GenericJoystick` pad entity (the entity that owns
/// `JoystickDragState` + `JoystickResendTimer`). Used to pause the 10 Hz
/// resend timer and reset drag state when impulse engages, so stale
/// `last_dx`/`last_dy` can't leak through the impulse barrier.
#[derive(Component)]
struct HelmJoystickPadEntity;

/// Marks the column overlay that hosts the cancel button + progress bar
/// shown in place of the joystick while the impulse drive is charging or
/// active. Toggled as a single unit so children stay laid out.
#[derive(Component)]
struct HelmImpulseOverlay;

/// Marks the "Cancel Impulse" button shown in place of the joystick while
/// the impulse drive is charging or active.
#[derive(Component)]
struct HelmCancelImpulseButton;

/// Marks the wrapper of the impulse charging progress bar (hidden when idle).
#[derive(Component)]
struct HelmImpulseProgressBar;

/// Marks the fill node of the impulse charging progress bar.
#[derive(Component)]
struct HelmImpulseProgressFill;

/// Marks the text node showing the charging countdown / "ENGAGED" status.
#[derive(Component)]
struct HelmImpulseStatusText;

// ── Plugin ───────────────────────────────────────────────────────────

pub struct HelmPanelPlugin;

impl Plugin for HelmPanelPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HelmTickTimer(Timer::from_seconds(
            0.1,
            TimerMode::Repeating,
        )))
        .init_resource::<HelmRadarEntities>()
        .add_systems(
            Update,
            (
                spawn_phone_helm_ui.run_if(not(resource_exists::<PhoneHelmSpawned>)),
                respawn_helm_on_orientation_change,
                toggle_helm_panel_visibility,
                update_helm_readouts,
                sync_helm_radar_range,
                bridge_client_sim_to_radar_entities,
                refresh_helm_impulse_state,
            ),
        );
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
        *vis = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    if !visible {
        // Send zero input when panel is hidden
        outbound.write(OutboundClientMessage(ClientMessage::HelmInput {
            thrust: 0.0,
            steering: 0.0,
        }));
    }
}

// ── Joystick observer ────────────────────────────────────────────────

fn on_helm_joystick_moved(
    trigger: On<JoystickMoved>,
    ship_view: Res<ShipView>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    // Suppress joystick output (including 10 Hz resends) while the impulse
    // drive is charging or active — the autopilot is steering the ship and
    // a stale knob value must not override it, nor snap the ship the
    // instant the drive disengages.
    if !should_send_helm_input(ship_view.impulse_charge_progress) {
        return;
    }
    let moved = trigger.event();
    let thrust = -moved.dy;
    let steering = moved.dx;
    outbound.write(OutboundClientMessage(ClientMessage::HelmInput {
        thrust,
        steering,
    }));
}

// ── Button observers ─────────────────────────────────────────────────

fn on_on_screen_button_pressed(
    _trigger: On<ButtonPressed>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    outbound.write(OutboundClientMessage(ClientMessage::SetView {
        mode: ViewMode::Radar,
    }));
}

fn on_impulse_button_pressed(
    _trigger: On<ButtonPressed>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    outbound.write(OutboundClientMessage(ClientMessage::StartImpulseCharge));
}

fn on_cancel_impulse_pressed(
    _trigger: On<ButtonPressed>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    outbound.write(OutboundClientMessage(ClientMessage::CancelImpulse));
}

// ── Impulse overlay state ────────────────────────────────────────────

/// Each frame: derive joystick / cancel-overlay visibility, progress-bar
/// fill, status text, and joystick-resend gating from
/// `ShipView.impulse_charge_progress`.
///
/// Compare-then-write on `Visibility` and `Node.width` to avoid change-
/// detection churn each frame.
///
/// On the rising edge (Idle → Charging/Active) we also reset the pad's
/// `JoystickDragState` so stale `last_dx`/`last_dy` can't be resent (the
/// `paused` gate plugs the periodic resend, and `reset_joystick_drag`
/// plugs the value that would have been resent).
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
fn refresh_helm_impulse_state(
    ship_view: Res<ShipView>,
    lobby: Res<LobbyState>,
    mut prev_progress: Local<f32>,
    mut joystick: Query<
        &mut Visibility,
        (
            With<HelmJoystickPad>,
            Without<HelmImpulseOverlay>,
            Without<HelmImpulseProgressFill>,
        ),
    >,
    mut overlay: Query<
        &mut Visibility,
        (
            With<HelmImpulseOverlay>,
            Without<HelmJoystickPad>,
            Without<HelmImpulseProgressFill>,
        ),
    >,
    mut fill: Query<
        &mut Node,
        (
            With<HelmImpulseProgressFill>,
            Without<HelmJoystickPad>,
            Without<HelmImpulseOverlay>,
        ),
    >,
    mut status: Query<&mut Text, With<HelmImpulseStatusText>>,
    mut pad_state: Query<
        (&mut JoystickDragState, &mut JoystickResendTimer),
        With<HelmJoystickPadEntity>,
    >,
) {
    let progress = ship_view.impulse_charge_progress;
    let (joystick_visible, cancel_visible) = impulse_ui_visibility(progress);
    let to_vis = |b: bool| {
        if b {
            Visibility::Visible
        } else {
            Visibility::Hidden
        }
    };

    // Visibility: compare-then-write to avoid Changed<Visibility> spam.
    let want_joystick = to_vis(joystick_visible);
    for mut v in joystick.iter_mut() {
        if *v != want_joystick {
            *v = want_joystick;
        }
    }
    let want_overlay = to_vis(cancel_visible);
    for mut v in overlay.iter_mut() {
        if *v != want_overlay {
            *v = want_overlay;
        }
    }

    // Progress bar fill width: compare-then-write.
    let pct = progress.clamp(0.0, 1.0) * 100.0;
    let want_width = Val::Percent(pct);
    for mut node in fill.iter_mut() {
        if node.width != want_width {
            node.width = want_width;
        }
    }

    // Status text: pull charge_duration from the server-supplied ship
    // client config when available; fall back to the wire default.
    let charge_duration = if lobby.ship_config.impulse_charge_duration > 0.0 {
        lobby.ship_config.impulse_charge_duration
    } else {
        crate::impulse::IMPULSE_CHARGE_DURATION
    };
    let want_text = format_impulse_status(progress, charge_duration).unwrap_or_default();
    for mut text in status.iter_mut() {
        if text.0 != want_text {
            text.0 = want_text.clone();
        }
    }

    // Joystick gate: pause the 10 Hz resend whenever impulse is non-zero,
    // and on the rising edge zero the cached drag so the gate's plug
    // doesn't merely freeze a stale value in place.
    let now_active = progress > 0.0;
    let was_active = *prev_progress > 0.0;
    let want_pause = now_active;
    for (mut drag, mut timer) in pad_state.iter_mut() {
        if timer.paused != want_pause {
            timer.paused = want_pause;
        }
        if now_active && !was_active {
            reset_joystick_drag(&mut drag);
        }
    }
    *prev_progress = progress;
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
            HelmReadoutKind::Hdg => {
                ReadoutValue(format!("HDG {}", yaw_to_heading(ship_view.ship_yaw)))
            }
            HelmReadoutKind::Spd => ReadoutValue(format!("SPD {:.0}", ship_view.forward_speed)),
            HelmReadoutKind::X => ReadoutValue(format!("X {:.0}", ship_view.ship_x)),
            HelmReadoutKind::Z => ReadoutValue(format!("Z {:.0}", ship_view.ship_z)),
        };
    }
}

/// Keeps the **helm** `GenericRadarWidget.range` in sync with the
/// server-provided `LobbyState.ship_config.helm_radar_range`.
///
/// The query is intentionally restricted to `With<HelmRadarWidget>` so that
/// other consoles' radar ranges (sensors = 200 units, navigation = auto-scaled)
/// are not accidentally overwritten.
fn sync_helm_radar_range(
    lobby: Res<LobbyState>,
    mut radars: Query<&mut GenericRadarWidget, With<HelmRadarWidget>>,
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

/// Build `OnRadar` / `RadarAppearance` for the `GenericRadar` widget.
fn bridge_client_sim_to_radar_entities(
    mut commands: Commands,
    sim: Res<ClientSimState>,
    ship_view: Res<ShipView>,
    mut radar: ResMut<HelmRadarEntities>,
) {
    // Manage the RadarCenter entity.
    //
    // The centre entity doubles as the player-ship blip: we attach
    // `OnRadar(PlayerShip) + RadarAppearance` and a Transform at the ship's
    // world position.  Using `PlayerShip` (not `Ship`) lets filters that want
    // "player ship but not NPC ships" include `PlayerShip` without also showing
    // every hostile.  In `ShipRelative` orientation it projects to the radar
    // centre (0, 0).
    let ship_appearance = RadarAppearance {
        icon: RadarIcon::Ship,
        world_size: 6.0,
        color: Color::srgb(0.95, 0.95, 1.0),
        region_colour: None,
        region_shape: None,
    };
    let ship_yaw = ship_view.ship_yaw;
    let ship_transform = Transform::from_xyz(ship_view.ship_x, 0.0, ship_view.ship_z)
        .with_rotation(Quat::from_rotation_y(ship_yaw));
    let ship_global = GlobalTransform::from(ship_transform);
    let _center_entity = match radar.center {
        Some(e) => {
            commands.entity(e).insert((
                RadarCenter {
                    world_x: ship_view.ship_x,
                    world_z: ship_view.ship_z,
                    yaw: ship_yaw,
                },
                OnRadar(RadarLayer::PlayerShip),
                ship_appearance,
                ship_transform,
                ship_global,
            ));
            e
        }
        None => {
            let e = commands
                .spawn((
                    RadarCenter {
                        world_x: ship_view.ship_x,
                        world_z: ship_view.ship_z,
                        yaw: ship_yaw,
                    },
                    OnRadar(RadarLayer::PlayerShip),
                    ship_appearance,
                    ship_transform,
                    ship_global,
                ))
                .id();
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

        let Some(layer) = tags_to_radar_layer(&snapshot.tags) else {
            continue;
        };

        let entity_yaw = snapshot.yaw.unwrap_or(0.0);
        let colour = snapshot.colour.map(|c| Color::srgb(c[0], c[1], c[2]));

        if layer == RadarLayer::Region || layer == RadarLayer::AsteroidField {
            // ── Region / field entity: render as shape ────────────────────────
            let region_colour = colour.unwrap_or(default_layer_colour(layer));
            let region_shape = region_shape_from_snapshot(snapshot);
            let world_size = snapshot
                .radar_world_size
                .or(Some(snapshot.radius_or_zero()))
                .filter(|s| *s > 0.0)
                .unwrap_or(4.0);
            let appearance = RadarAppearance {
                icon: layer_to_icon(layer),
                world_size,
                color: region_colour,
                region_colour: Some(region_colour),
                region_shape,
            };
            let t = Transform::from_xyz(snapshot.x(), 0.0, snapshot.z())
                .with_rotation(Quat::from_rotation_y(entity_yaw));
            if let Some(existing) = radar.blips.get(uuid) {
                commands.entity(*existing).insert((
                    OnRadar(layer),
                    appearance,
                    t,
                    GlobalTransform::from(t),
                ));
            } else {
                let blip = commands
                    .spawn((
                        OnRadar(layer),
                        appearance,
                        t,
                        GlobalTransform::from(t),
                    ))
                    .id();
                radar.blips.insert(uuid.clone(), blip);
            }
        } else {
            // ── Point entity: render as icon ──────────────────────────────────
            let default_color = default_layer_colour(layer);
            let icon = layer_to_icon(layer);
            let world_size = snapshot
                .radar_world_size
                .or(Some(snapshot.radius_or_zero()))
                .filter(|s| *s > 0.0)
                .unwrap_or(4.0);
            let appearance = RadarAppearance {
                icon,
                world_size,
                color: colour.unwrap_or(default_color),
                region_colour: None,
                region_shape: None,
            };
            let t = Transform::from_xyz(snapshot.x(), 0.0, snapshot.z())
                .with_rotation(Quat::from_rotation_y(entity_yaw));
            if let Some(existing) = radar.blips.get(uuid) {
                commands.entity(*existing).insert((
                    OnRadar(layer),
                    appearance,
                    t,
                    GlobalTransform::from(t),
                ));
            } else {
                let blip = commands
                    .spawn((
                        OnRadar(layer),
                        appearance,
                        t,
                        GlobalTransform::from(t),
                    ))
                    .id();
                radar.blips.insert(uuid.clone(), blip);
            }
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

/// Spawns the phone helm panel using `ConsoleShell::spawn` (PRD #346).
fn spawn_phone_helm_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    old_panel: Query<Entity, With<HelmPanel>>,
    old_help: Query<(Entity, &crate::client::elements::HelpOverlay)>,
    orientation: Option<Res<DeviceOrientation>>,
) {
    let Some(assets) = assets else { return };
    let is_landscape = crate::phone_border::framing::is_landscape(orientation.as_deref());

    for entity in old_panel.iter() {
        commands.entity(entity).despawn();
    }
    // Despawn any stale Helm help-overlay from a previous spawn (e.g. an
    // orientation respawn) before ConsoleShell::spawn creates a fresh one.
    for (entity, overlay) in old_help.iter() {
        if overlay.0 == crate::client::elements::HelpPanel::Helm {
            commands.entity(entity).despawn();
        }
    }

    commands.insert_resource(PhoneHelmSpawned);

    let shell = ConsoleShell::spawn(
        &mut commands,
        assets.helm_panel_bg.clone(),
        is_landscape,
        crate::client::elements::HelpPanel::Helm,
        |commands: &mut Commands, primary: Entity| {
            fill_helm_radar(commands, primary, &assets, is_landscape);
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

fn fill_helm_radar(
    commands: &mut Commands,
    container: Entity,
    assets: &PhoneAssets,
    is_landscape: bool,
) {
    let dim = Color::srgb(0.55, 0.70, 1.0);
    let readout_visuals =
        || StateVisuals::from_colors(dim, dim, dim, dim, Color::srgb(0.3, 0.4, 0.6));
    let display = |s: f32| TextFont {
        font: assets.font_display.clone(),
        font_size: s,
        ..default()
    };

    // ── Column wrapper to stack radar + buttons vertically ──
    let col = commands
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            width: Val::Percent(90.0),
            height: Val::Percent(90.0),
            row_gap: Val::Px(8.0),
            ..default()
        })
        .id();
    commands.entity(container).add_child(col);

    // ── Radar ──
    //
    // Filter includes PlayerShip (the local ship, added by the bridge as
    // RadarLayer::PlayerShip) and AsteroidField (field regions, now a
    // distinct layer from individual Asteroid blips).
    let radar_filter = RadarFilter(std::collections::HashSet::from([
        RadarLayer::PlayerShip,
        RadarLayer::Ship,
        RadarLayer::Asteroid,
        RadarLayer::AsteroidField,
        RadarLayer::Station,
        RadarLayer::Missile,
        RadarLayer::Planet,
        RadarLayer::Star,
    ]));
    // Layering (back → front, per user direction):
    //   1. radar-surround.png  (outer frame / tick marks) — passed as `bg_image`
    //   2. radar-bg.png        (inner dial face)          — passed as `overlay_image`
    //   3. blips rendered as child UI nodes (icons) over both.
    //
    // The arg names on `GenericRadar::spawn` describe Z-order (bg = behind,
    // overlay = in front) rather than asset role, so we deliberately pass
    // surround as the back layer and bg as the front layer.
    let radar = GenericRadar::spawn(
        commands,
        HELM_RADAR_RANGE_FALLBACK,
        OrientationMode::ShipRelative,
        radar_filter,
        Some(assets.radar_surround.clone()),
        Some(assets.radar_bg.clone()),
        RadarClipMode::Circle,
    );
    // HelmRadarWidget marks this widget so sync_helm_radar_range only
    // updates it, not other consoles' radar widgets.
    commands.entity(radar).insert(HelmRadarWidget);
    // Override the default Val::Percent(100.0)-width sizing from
    // GenericRadar::spawn so the radar squares-fit the parent slot:
    //   landscape → constrain by height (parent is wider than tall)
    //   portrait  → constrain by width  (parent is taller than wide)
    // aspect_ratio: 1.0 derives the other axis from the constrained one.
    commands.entity(radar).insert(Node {
        width: if is_landscape {
            Val::Auto
        } else {
            Val::Percent(100.0)
        },
        height: if is_landscape {
            Val::Percent(100.0)
        } else {
            Val::Auto
        },
        aspect_ratio: Some(1.0),
        position_type: PositionType::Relative,
        ..default()
    });
    commands.entity(col).add_child(radar);

    // HDG readout
    let hdg = TextReadout::spawn(commands, "HDG", readout_visuals());
    commands.entity(hdg).insert((
        HelmReadoutKind::Hdg,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(2.0),
            top: Val::Px(2.0),
            ..default()
        },
    ));
    commands.entity(radar).add_child(hdg);

    // SPD readout
    let spd = TextReadout::spawn(commands, "SPD", readout_visuals());
    commands.entity(spd).insert((
        HelmReadoutKind::Spd,
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(2.0),
            top: Val::Px(2.0),
            ..default()
        },
    ));
    commands.entity(radar).add_child(spd);

    // X readout
    let x_read = TextReadout::spawn(commands, "X", readout_visuals());
    commands.entity(x_read).insert((
        HelmReadoutKind::X,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(2.0),
            bottom: Val::Px(2.0),
            ..default()
        },
    ));
    commands.entity(radar).add_child(x_read);

    // Z readout
    let z_read = TextReadout::spawn(commands, "Z", readout_visuals());
    commands.entity(z_read).insert((
        HelmReadoutKind::Z,
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(2.0),
            bottom: Val::Px(2.0),
            ..default()
        },
    ));
    commands.entity(radar).add_child(z_read);

    // ── Buttons row ──
    let buttons_row = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            column_gap: Val::Px(8.0),
            ..default()
        })
        .id();

    // ON SCREEN button with button-normal PNG visuals
    let on_screen_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect {
            width: 120.0,
            height: 32.0,
        },
        StateVisuals {
            idle: Visual {
                image: Some(assets.btn_normal_idle.clone()),
                color: Color::NONE,
            },
            hover: Visual {
                image: Some(assets.btn_normal_hover.clone()),
                color: Color::NONE,
            },
            active: Visual {
                image: Some(assets.btn_normal_active.clone()),
                color: Color::NONE,
            },
            press: Visual {
                image: Some(assets.btn_normal_press.clone()),
                color: Color::NONE,
            },
            disabled: Visual {
                image: None,
                color: Color::srgba(0.08, 0.08, 0.15, 0.5),
            },
        },
        None,
    );
    commands.entity(on_screen_btn).with_children(|btn| {
        btn.spawn((
            Text::new("ON SCREEN"),
            display(12.0),
            TextColor(Color::srgb(0.93, 0.93, 1.0)),
        ));
    });
    commands
        .entity(on_screen_btn)
        .observe(on_on_screen_button_pressed);
    commands.entity(buttons_row).add_child(on_screen_btn);

    // IMPULSE button with impulse PNG visuals
    let impulse_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect {
            width: 120.0,
            height: 32.0,
        },
        StateVisuals {
            idle: Visual {
                image: Some(assets.impulse_ready.clone()),
                color: Color::NONE,
            },
            hover: Visual {
                image: Some(assets.impulse_hover.clone()),
                color: Color::NONE,
            },
            active: Visual {
                image: Some(assets.impulse_active.clone()),
                color: Color::NONE,
            },
            press: Visual {
                image: Some(assets.impulse_press.clone()),
                color: Color::NONE,
            },
            disabled: Visual {
                image: None,
                color: Color::srgba(0.05, 0.10, 0.20, 0.5),
            },
        },
        None,
    );
    commands.entity(impulse_btn).with_children(|btn| {
        btn.spawn((
            Text::new("IMPULSE"),
            display(12.0),
            TextColor(Color::srgb(0.5, 0.8, 1.0)),
        ));
    });
    commands
        .entity(impulse_btn)
        .observe(on_impulse_button_pressed);
    commands.entity(buttons_row).add_child(impulse_btn);

    commands.entity(col).add_child(buttons_row);
}

fn fill_helm_joystick(commands: &mut Commands, container: Entity, assets: &PhoneAssets) {
    let dim = Color::srgb(0.55, 0.70, 1.0);
    let muted = Color::srgb(0.6, 0.7, 0.73);
    let mono = |s: f32| TextFont {
        font: assets.font_mono.clone(),
        font_size: s,
        ..default()
    };

    // ── Column wrapper to stack joystick elements vertically ──
    let col = commands
        .spawn((
            HelmJoystickPad,
            Node {
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(4.0),
                ..default()
            },
        ))
        .id();
    commands.entity(container).add_child(col);

    // Directional indicators
    let fwd = commands
        .spawn((Text::new("▲"), mono(16.0), TextColor(dim), Node::default()))
        .id();
    commands.entity(col).add_child(fwd);

    let joystick_center_row = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(4.0),
            ..default()
        })
        .id();
    commands.entity(col).add_child(joystick_center_row);

    // Left directional chevron
    let left_arrow = commands
        .spawn((Text::new("◄"), mono(14.0), TextColor(dim), Node::default()))
        .id();
    commands.entity(joystick_center_row).add_child(left_arrow);

    // Joystick with PNG visuals
    let joystick_knob_visuals = StateVisuals {
        idle: Visual {
            image: Some(assets.joystick_knob_idle.clone()),
            color: Color::NONE,
        },
        hover: Visual {
            image: Some(assets.joystick_knob_hover.clone()),
            color: Color::NONE,
        },
        active: Visual {
            image: Some(assets.joystick_knob_active.clone()),
            color: Color::NONE,
        },
        press: Visual {
            image: Some(assets.joystick_knob_press.clone()),
            color: Color::NONE,
        },
        disabled: Visual {
            image: None,
            color: Color::srgba(0.05, 0.05, 0.10, 0.5),
        },
    };
    let pad = GenericJoystick::spawn(
        commands,
        HELM_PAD_SIZE,
        Some(assets.joystick_pad_idle.clone()),
        Some(assets.joystick_knob_idle.clone()),
        joystick_knob_visuals,
    );
    commands.entity(pad).observe(on_helm_joystick_moved);
    commands.entity(pad).insert(HelmJoystickPadEntity);
    commands.entity(joystick_center_row).add_child(pad);

    // Right directional chevron
    let right_arrow = commands
        .spawn((Text::new("►"), mono(14.0), TextColor(dim), Node::default()))
        .id();
    commands.entity(joystick_center_row).add_child(right_arrow);

    let aft = commands
        .spawn((Text::new("▼"), mono(16.0), TextColor(dim), Node::default()))
        .id();
    commands.entity(col).add_child(aft);

    // FWD/REV labels
    let fwd_rev_row = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            column_gap: Val::Px(24.0),
            ..default()
        })
        .id();
    commands.entity(fwd_rev_row).with_children(|row| {
        row.spawn((Text::new("FWD"), mono(9.0), TextColor(muted)));
        row.spawn((Text::new("REV"), mono(9.0), TextColor(muted)));
    });
    commands.entity(col).add_child(fwd_rev_row);

    // PORT/STBD labels
    let port_stbd_row = commands
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            column_gap: Val::Px(32.0),
            ..default()
        })
        .id();
    commands.entity(port_stbd_row).with_children(|row| {
        row.spawn((Text::new("PORT"), mono(9.0), TextColor(muted)));
        row.spawn((Text::new("STBD"), mono(9.0), TextColor(muted)));
    });
    commands.entity(col).add_child(port_stbd_row);

    // ── Impulse overlay: visible only while the impulse drive is charging
    // or active. Sits as a sibling of `col` (the joystick column) inside
    // `container`; visibility is toggled mutually-exclusively in
    // `refresh_helm_impulse_visibility`.
    let overlay = commands
        .spawn((
            HelmImpulseOverlay,
            Node {
                width: Val::Px(HELM_PAD_SIZE),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(10.0),
                ..default()
            },
            Visibility::Hidden,
        ))
        .id();
    commands.entity(container).add_child(overlay);

    // Cancel button (color-only, no PNG art).
    let cancel_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect {
            width: 160.0,
            height: 44.0,
        },
        StateVisuals {
            idle: Visual {
                image: None,
                color: Color::srgba(0.45, 0.05, 0.05, 0.85),
            },
            hover: Visual {
                image: None,
                color: Color::srgba(0.65, 0.10, 0.10, 0.95),
            },
            active: Visual {
                image: None,
                color: Color::srgba(0.65, 0.10, 0.10, 0.95),
            },
            press: Visual {
                image: None,
                color: Color::srgba(0.85, 0.15, 0.15, 1.0),
            },
            disabled: Visual {
                image: None,
                color: Color::srgba(0.20, 0.05, 0.05, 0.5),
            },
        },
        None,
    );
    commands
        .entity(cancel_btn)
        .insert(HelmCancelImpulseButton)
        .with_children(|btn| {
            btn.spawn((
                Text::new("CANCEL IMPULSE"),
                mono(12.0),
                TextColor(Color::srgb(1.0, 0.85, 0.85)),
            ));
        })
        .observe(on_cancel_impulse_pressed);
    commands.entity(overlay).add_child(cancel_btn);

    // Charge progress bar: wrapper + fill node. Fill width is driven each
    // frame by `refresh_helm_impulse_visibility` from
    // `ShipView.impulse_charge_progress`.
    let bar = commands
        .spawn((
            HelmImpulseProgressBar,
            Node {
                width: Val::Px(HELM_PAD_SIZE),
                height: Val::Px(10.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.10, 0.15, 0.20, 0.85)),
        ))
        .id();
    let fill = commands
        .spawn((
            HelmImpulseProgressFill,
            Node {
                width: Val::Percent(0.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(Color::srgb(0.40, 0.85, 1.0)),
        ))
        .id();
    commands.entity(bar).add_child(fill);
    commands.entity(overlay).add_child(bar);

    // Status text: "X.X / Y.Y s" while charging, "ENGAGED" once active.
    // Empty by default; driven by `refresh_helm_impulse_state` via the
    // pure `format_impulse_status` helper.
    let status = commands
        .spawn((
            HelmImpulseStatusText,
            Text::new(""),
            mono(11.0),
            TextColor(Color::srgb(0.85, 0.95, 1.0)),
            Node::default(),
        ))
        .id();
    commands.entity(overlay).add_child(status);
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
    // Skip the first-frame `is_changed` that fires when the resource is
    // freshly inserted — there is no helm panel to respawn yet, and treating
    // the insert as a "change" would orphan the just-spawned panel root,
    // since `despawn_related::<Children>` keeps the marked entity alive
    // while `spawn_phone_helm_ui` then creates a second `HelmPanel`.
    if orientation.is_added() {
        return;
    }
    // Despawn the panel root entirely (along with all children) so the next
    // `spawn_phone_helm_ui` produces exactly one `HelmPanel`, not two.
    for entity in panel.iter() {
        commands.entity(entity).despawn();
    }
    // Clear stale entity IDs so bridge systems don't command dead entities.
    if let Some(center) = radar.center.take() {
        commands.entity(center).despawn();
    }
    for (_, entity) in radar.blips.drain() {
        commands.entity(entity).despawn();
    }
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

    fn in_progress_helm_lobby(token: &str) -> LobbyState {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player(token, vec![Console::Helm])],
        )));
        s
    }

    fn no_tab() -> ActiveConsole {
        ActiveConsole(None)
    }
    fn tab(c: Console) -> ActiveConsole {
        ActiveConsole(Some(c))
    }

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
