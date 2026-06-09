//! Client-side Helm Panel plugin â€” migrated to `src/gui/` library widgets.
//!
//! Owns all helm console UI: joystick (`GenericJoystick`), radar (`GenericRadar`),
//! buttons (`GuiButton`), readouts (`TextReadout`), panel visibility, and 10 Hz
//! input resend. No per-button marker-component query systems remain.

use bevy::prelude::*;

use crate::client::console_shell::ConsoleShell;
use crate::client_app::{ClientSet, HelmPanel, OutboundClientMessage};
use crate::client_lobby::{ActiveConsole, LobbyState, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::gui::{
    spawn_gui_button, bridge_sim_to_radar, ButtonPressed, ButtonSize, ConsoleRadar,
    GenericJoystick, GenericRadar, JoystickMoved, OrientationMode,
    RadarBlipMap, RadarCenterPose, RadarClipMode, RadarFilter, ReadoutValue, StateVisuals,
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

// â”€â”€ Pure helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

// â”€â”€ Resources â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// 10 Hz resend timer for the helm joystick.
///
/// The `GenericJoystick` has its own 10 Hz resend via `JoystickResendTimer`
/// but the helm plugin keeps this resource for backward-compat systems
/// that may reference it (e.g. `toggle_helm_panel_visibility`).
#[derive(Resource)]
pub struct HelmTickTimer(pub Timer);

// â”€â”€ Readout kind discriminator â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Distinguishes which helm readout a `TextReadout` root belongs to.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
enum HelmReadoutKind {
    Hdg,
    Spd,
    X,
    Z,
}

// â”€â”€ Constants â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Diameter of the joystick pad in logical pixels.
pub const HELM_PAD_SIZE: f32 = 200.0;

/// Fallback radar range in world units, used when the server has not yet
/// sent a `Welcome` with a `ShipClientConfig.helm_radar_range`. The real
/// runtime value comes from `LobbyState.ship_config.helm_radar_range`,
/// sourced from `[helm_console.radar] range` in `player_ship.toml`.
pub const HELM_RADAR_RANGE_FALLBACK: f32 = 250.0;

/// Convert ship yaw (radians, CW from -Z) to a 3-digit heading string
/// (degrees, 0–359, 0 = North / ship-forward; positive yaw → East / 090°).
pub fn yaw_to_heading(yaw_rad: f32) -> String {
    let heading = yaw_rad.to_degrees().rem_euclid(360.0);
    format!("{:03}°", heading.round() as u32)
}

// â”€â”€ Impulse UI markers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

/// Marks the "IMPULSE" button in the radar column (visible when idle).
#[derive(Component)]
struct HelmRadarImpulseButton;

/// Marks the "CANCEL IMPULSE" button in the radar column (visible when charging/active).
#[derive(Component)]
struct HelmRadarCancelButton;

// â”€â”€ Plugin â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

pub struct HelmPanelPlugin;

impl Plugin for HelmPanelPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HelmTickTimer(Timer::from_seconds(
            0.1,
            TimerMode::Repeating,
        )))
        .add_systems(
            Update,
            (
                spawn_phone_helm_ui.run_if(not(resource_exists::<PhoneHelmSpawned>)),
                respawn_helm_on_orientation_change,
                // ConsoleUpdate runs after MessageProcessing, so ShipView
                // is already current when these systems execute.
                toggle_helm_panel_visibility.in_set(ClientSet::ConsoleUpdate),
                update_helm_readouts.in_set(ClientSet::ConsoleUpdate),
                bridge_helm_radar,
                refresh_helm_impulse_state.in_set(ClientSet::ConsoleUpdate),
            ),
        );
    }
}

// â”€â”€ Visibility system â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn toggle_helm_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<HelmPanel>>,
    mut outbound: MessageWriter<OutboundClientMessage>,
    mut was_visible: Local<bool>,
) {
    let visible = helm_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    // Send zero input only on the falling edge (visible → hidden) so we don't
    // spam the server every frame while the panel is already hidden.
    if *was_visible && !visible {
        outbound.write(OutboundClientMessage(ClientMessage::HelmInput {
            thrust: 0.0,
            steering: 0.0,
        }));
    }
    *was_visible = visible;
}

// â”€â”€ Joystick observer â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

fn on_helm_joystick_moved(
    trigger: On<JoystickMoved>,
    ship_view: Res<ShipView>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    // Suppress joystick output (including 10 Hz resends) while the impulse
    // drive is charging or active â€” the autopilot is steering the ship and
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

// â”€â”€ Button observers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

// â”€â”€ Impulse overlay state â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Each frame: derive joystick / cancel-overlay visibility, progress-bar
/// fill, status text, and joystick-resend gating from
/// `ShipView.impulse_charge_progress`.
///
/// Compare-then-write on `Visibility` and `Node.width` to avoid change-
/// detection churn each frame.
///
/// On the rising edge (Idle â†’ Charging/Active) we also reset the pad's
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
    mut radar_impulse_btn: Query<
        &mut Visibility,
        (
            With<HelmRadarImpulseButton>,
            Without<HelmRadarCancelButton>,
            Without<HelmJoystickPad>,
            Without<HelmImpulseOverlay>,
        ),
    >,
    mut radar_cancel_btn: Query<
        &mut Visibility,
        (
            With<HelmRadarCancelButton>,
            Without<HelmRadarImpulseButton>,
            Without<HelmJoystickPad>,
            Without<HelmImpulseOverlay>,
        ),
    >,
    mut pad_state: Query<
        (&mut JoystickDragState, &mut JoystickResendTimer),
        With<HelmJoystickPadEntity>,
    >,
) {
    let progress = ship_view.impulse_charge_progress;
    let (joystick_visible, cancel_visible) = impulse_ui_visibility(progress);
    let to_vis = |b: bool| {
        if b {
            Visibility::Inherited
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

    // Radar column impulse/cancel button visibility:
    // impulse button visible when idle, cancel visible when charging/active.
    let want_impulse_btn = to_vis(joystick_visible);
    for mut v in radar_impulse_btn.iter_mut() {
        if *v != want_impulse_btn {
            *v = want_impulse_btn;
        }
    }
    let want_radar_cancel = to_vis(cancel_visible);
    for mut v in radar_cancel_btn.iter_mut() {
        if *v != want_radar_cancel {
            *v = want_radar_cancel;
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

    // Joystick gate: the WASM joystick's 10 Hz resend is always paused because
    // the HTML iframe intercepts every pointer event for the helm panel — the
    // WASM widget is never dragged and its `last_dx`/`last_dy` default to 0.
    // Without this gate, the 10 Hz zero-heartbeat overwrites real thrust values
    // sent by the HTML joystick and the ship stops moving.
    //
    // On the rising edge of impulse (Idle → Charging/Active) we also zero the
    // cached drag state so any stale knob value cannot leak through once the
    // drive disengages.  Immediate drag events still fire normally; only the
    // periodic heartbeat resend is suppressed.
    let now_active = progress > 0.0;
    let was_active = *prev_progress > 0.0;
    for (mut drag, mut timer) in pad_state.iter_mut() {
        if !timer.paused {
            timer.paused = true;
        }
        if now_active && !was_active {
            reset_joystick_drag(&mut drag);
        }
    }
    *prev_progress = progress;
}

// â”€â”€ Readout update system â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Each frame: write HDG / SPD / X / Z readout values from `ShipView`.
/// No `is_changed()` gate â€” readouts must update every frame so that small
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

// â”€â”€ Radar entity bridge â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// Reconcile `ClientSimState.world.entities` into ECS radar blips for the
/// helm `GenericRadar` widget. All loop logic lives in
/// [`bridge_sim_to_radar`]; this is the per-console wiring.
fn bridge_helm_radar(
    mut commands: Commands,
    sim: Res<ClientSimState>,
    ship_view: Res<ShipView>,
    mut q: Query<(Entity, &ConsoleRadar, &mut RadarBlipMap)>,
) {
    let Some((widget, _, mut map)) =
        q.iter_mut().find(|(_, c, _)| **c == ConsoleRadar::Helm)
    else {
        return;
    };
    bridge_sim_to_radar(
        &mut commands,
        widget,
        &mut map,
        RadarCenterPose {
            x: ship_view.ship_x,
            z: ship_view.ship_z,
            yaw: ship_view.ship_yaw,
        },
        &sim.world.entities,
    );
}




// â”€â”€ Phone helm UI spawn â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

    // Insert HelmPanel marker on the root for visibility queries.
    // Start hidden; toggle_helm_panel_visibility reveals it when appropriate.
    commands.entity(shell.root).insert((HelmPanel, Visibility::Hidden));
}

// â”€â”€ Fill helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

    // â”€â”€ Column wrapper to stack radar + buttons vertically â”€â”€
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

    // â”€â”€ Radar â”€â”€
    //
    // Spawn with an empty filter â€” sync_helm_radar_range will populate it
    // from lobby.ship_config.helm_radar_shows once the Welcome arrives.
    let radar_filter = RadarFilter(std::collections::HashSet::new());
    // Layering (back â†’ front, per user direction):
    //   1. radar-surround.png  (outer frame / tick marks) â€” passed as `bg_image`
    //   2. radar-bg.png        (inner dial face)          â€” passed as `overlay_image`
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
        560.0 / 640.0, // overlay_fraction: radar-bg (560px) centred within surround (640px)
        270.0 / 280.0, // face_fraction: measured circle radius (270px) / bg half (280px)
    );
    // Tag the widget with its console identity and attach a RadarBlipMap
    // so `bridge_helm_radar` can find it and reconcile blips into it.
    commands.entity(radar).insert((ConsoleRadar::Helm, RadarBlipMap::default()));
    // Override the default Val::Percent(100.0)-width sizing from
    // GenericRadar::spawn so the radar squares-fit the parent slot:
    //   landscape â†’ constrain by height (parent is wider than tall)
    //   portrait  â†’ constrain by width  (parent is taller than wide)
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

    // â”€â”€ Buttons row â”€â”€
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
    // IMPULSE button (visible when idle)
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
    );
    commands
        .entity(impulse_btn)
        .insert(HelmRadarImpulseButton)
        .with_children(|btn| {
            btn.spawn((
                Text::new("IMPULSE"),
                display(12.0),
                TextColor(Color::srgb(0.5, 0.8, 1.0)),
            ));
        })
        .observe(on_impulse_button_pressed);
    commands.entity(buttons_row).add_child(impulse_btn);

    // CANCEL IMPULSE button (visible when charging/active, replaces IMPULSE)
    let cancel_impulse_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect {
            width: 120.0,
            height: 32.0,
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
    );
    commands
        .entity(cancel_impulse_btn)
        .insert(HelmRadarCancelButton)
        .with_children(|btn| {
            btn.spawn((
                Text::new("CANCEL\nIMPULSE"),
                display(10.0),
                TextColor(Color::srgb(1.0, 0.85, 0.85)),
            ));
        })
        .observe(on_cancel_impulse_pressed);
    commands.entity(buttons_row).add_child(cancel_impulse_btn);

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

    // â”€â”€ Column wrapper to stack joystick elements vertically â”€â”€
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
        .spawn((Text::new("â–²"), mono(16.0), TextColor(dim), Node::default()))
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
        .spawn((Text::new("â—„"), mono(14.0), TextColor(dim), Node::default()))
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
        .spawn((Text::new("â–º"), mono(14.0), TextColor(dim), Node::default()))
        .id();
    commands.entity(joystick_center_row).add_child(right_arrow);

    let aft = commands
        .spawn((Text::new("â–¼"), mono(16.0), TextColor(dim), Node::default()))
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

    // â”€â”€ Impulse overlay: visible only while the impulse drive is charging
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

// â”€â”€ Orientation respawn â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// When `DeviceOrientation` changes, despawn the current `HelmPanel` and
/// remove the `PhoneHelmSpawned` resource so `spawn_phone_helm_ui` respawns
/// with the correct layout.
fn respawn_helm_on_orientation_change(
    orientation: Res<DeviceOrientation>,
    panel: Query<Entity, With<HelmPanel>>,
    mut commands: Commands,
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
    // `spawn_phone_helm_ui` produces exactly one `HelmPanel`, not two. The
    // widget's `RadarBlipMap` and its blip child entities are also destroyed
    // because they descend from the panel root.
    for entity in panel.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<PhoneHelmSpawned>();
}

// â”€â”€ Tests â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::messages::{Console, GamePhase, GameState, Player, ServerMessage, ShipClientConfig};
    use crate::stations_config::ShipStations;
    use std::collections::HashMap;
    use std::f32::consts::{FRAC_PI_4, TAU};

    // â”€â”€ helm_panel_visible â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

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

    // â”€â”€ yaw_to_heading â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

    #[test]
    fn yaw_zero_is_heading_000() {
        assert_eq!(yaw_to_heading(0.0), "000Â°");
    }

    #[test]
    fn yaw_negative_quarter_turn_is_heading_045() {
        assert_eq!(yaw_to_heading(-FRAC_PI_4), "045Â°");
    }

    #[test]
    fn yaw_pi_is_heading_180() {
        assert_eq!(yaw_to_heading(std::f32::consts::PI), "180Â°");
    }

    #[test]
    fn yaw_negative_yaw_wraps_correctly() {
        let h = yaw_to_heading(-0.5);
        assert_eq!(h, "029Â°");
    }

    #[test]
    fn yaw_2pi_wraps_to_000() {
        assert_eq!(yaw_to_heading(TAU), "000Â°");
    }

    #[test]
    fn yaw_negative_angle_always_positive_heading() {
        let h = yaw_to_heading(-TAU);
        assert_eq!(h, "000Â°");
        let h2 = yaw_to_heading(-0.1);
        assert!(!h2.starts_with('-'));
    }
}
