//! Client-side Helm Panel plugin.
//!
//! Owns all helm console UI: compass-ring radar, joystick thumbstick,
//! panel visibility, 10 Hz input resend, On Screen button, and gizmo
//! radar overlay.
//!
//! Replaces the helm UI code that previously lived in `client/app.rs`
//! and the `phone_border/helm.rs` `HelmPanelPlugin`.
//!
//! Compiled only when the `client` Cargo feature is enabled.

use bevy::prelude::*;

use crate::client_app::{HelmPanel, OnScreenButton, RadarPanel, OutboundClientMessage};
use crate::client_helm::{HelmJoystickState, release, tick, drag};
use crate::client_lobby::{ActiveConsole, LobbyState, LocalPlayerToken};
use crate::client_sim::{on_screen_message, ClientSimState};
use crate::messages::{ClientMessage, Console, GamePhase, ViewMode};
use crate::phone_border::framing::{DeviceOrientation, PhoneAssets};
use crate::ship_view::ShipView;

// ── Pure helpers ─────────────────────────────────────────────────────

/// Decide whether the helm panel should be visible.
///
/// Rules:
/// 1. Game phase must be `InProgress`.
/// 2. The local player must hold `Console::Helm`.
/// 3. If holding **one console only**, show automatically.
/// 4. If holding **multiple consoles**, show only when `ActiveConsole`
///    is explicitly set to `Helm`.
pub fn helm_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    use crate::client_lobby::{LobbyView};
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
#[derive(Resource)]
pub struct HelmTickTimer(pub Timer);

// ── Constants ────────────────────────────────────────────────────────

/// Diameter of the joystick pad in logical pixels.
pub const HELM_PAD_SIZE: f32 = 200.0;

/// Radius of the knob disc, in pixels.
pub const HELM_KNOB_RADIUS: f32 = 24.0;

/// Diameter of the compass-ring radar in logical pixels.
pub const COMPASS_RADAR_DIAMETER: f32 = 280.0;

/// Effective max drag radius for the helm joystick.
pub fn helm_max_radius() -> f32 {
    (HELM_PAD_SIZE / 2.0) - HELM_KNOB_RADIUS - 2.0
}

// On Screen button background colours.
const ON_SCREEN_BG_IDLE:   Color = Color::srgb(0.13, 0.13, 0.27);
const ON_SCREEN_BG_ACTIVE: Color = Color::srgb(0.10, 0.40, 0.15);

// Gizmo radar colours.
const RADAR_OUTER_RING_COLOR: Color = Color::srgb(0.55, 0.70, 1.0);
const RADAR_MID_RING_COLOR:   Color = Color::srgb(0.30, 0.40, 0.65);
const RADAR_ASTEROID_COLOR:   Color = Color::srgb(0.85, 0.75, 0.45);
const RADAR_SHIP_COLOR:       Color = Color::srgb(0.95, 0.95, 1.0);

// Joystick colours.
const HELM_PAD_BG:          Color = Color::srgb(0.10, 0.10, 0.18);
const HELM_KNOB_BG_IDLE:    Color = Color::srgb(0.27, 0.27, 0.40);
const HELM_KNOB_BG_ACTIVE:  Color = Color::srgb(0.40, 0.40, 0.67);

// ── Marker components ────────────────────────────────────────────────

/// Marks the outermost container of the compass-ring radar.
#[derive(Component)]
pub struct PhoneCompassRadar;

/// Marks the rotating ring container driven by the ship's yaw.
#[derive(Component)]
pub struct PhoneCompassRing;

/// Marks a single tick mark on the compass ring.
#[derive(Component)]
pub struct PhoneCompassTick;

/// Marks the HDG corner readout text node.
#[derive(Component)]
pub struct PhoneHdgReadout;

/// Marks the SPD corner readout text node.
#[derive(Component)]
pub struct PhoneSpdReadout;

/// Marks the X corner readout text node.
#[derive(Component)]
pub struct PhoneXReadout;

/// Marks the Z corner readout text node.
#[derive(Component)]
pub struct PhoneZReadout;

/// Marks a range ring node.
#[derive(Component)]
pub struct PhoneRangeRing;

/// Marks the thumbstick outer ring visual.
#[derive(Component)]
pub struct PhoneThumbRing;

/// Marks the pad entity that captures pointer drag events.
#[derive(Component)]
pub struct PhoneHelmPad;

/// Marks the knob entity inside the phone thumbstick.
#[derive(Component)]
pub struct PhoneHelmKnob;

/// Marks the phone variant of the thrust/steering readout.
#[derive(Component)]
pub struct PhoneHelmReadout;

// ── One-shot marker ──────────────────────────────────────────────────

/// Marker resource set once the phone helm UI has been spawned.
#[derive(Resource)]
pub struct PhoneHelmSpawned;

/// Marks the "Impulse" button on the helm console.
#[derive(Component)]
pub struct HelmImpulseButton;

// ── Bearing tick model ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct BearingTick {
    pub angle_deg: f32,
    pub angle_rad: f32,
    pub label: String,
    pub is_major: bool,
}

/// Generate 36 bearing ticks at 10° intervals. Every 3rd tick (30° interval)
/// carries a text label (e.g. "00", "30", "60", … "330").
pub fn bearing_ticks() -> [BearingTick; 36] {
    std::array::from_fn(|i| {
        let degrees = i as f32 * 10.0;
        let is_major = i % 3 == 0;
        let label = if is_major {
            format!("{:02}", degrees as u32)
        } else {
            String::new()
        };
        BearingTick {
            angle_deg: degrees,
            angle_rad: degrees.to_radians(),
            label,
            is_major,
        }
    })
}

/// Compute 3 range-ring radii as fractions of a given radar-pixel radius.
pub fn range_ring_radii(radar_radius_px: f32) -> [f32; 3] {
    let third = radar_radius_px / 3.0;
    [third, third * 2.0, radar_radius_px]
}

/// Labels for the 3 range rings, matching `RANGE_RING_LABEL_DISTANCES`.
pub fn range_ring_labels() -> [String; 3] {
    ["200", "400", "600"].map(String::from)
}

/// Convert ship yaw (radians, CCW from +Z) to a 3-digit heading string
/// (degrees, 0–360, 0 = ship-forward = "north" on the compass).
pub fn yaw_to_heading(yaw_rad: f32) -> String {
    let heading = ((-yaw_rad).to_degrees()).rem_euclid(360.0);
    format!("{:03}°", heading.round() as u32)
}

/// Tracks the ship's computed speed from position deltas.
#[derive(Resource, Default)]
pub struct PhoneShipSpeed {
    pub speed: f32,
    prev_x: f32,
    prev_z: f32,
    initialized: bool,
}

// ── Plugin ───────────────────────────────────────────────────────────

/// Owns all helm console UI:
/// - Compass-ring radar + polished thumbstick (phone chrome)
/// - Panel visibility toggling
/// - 10 Hz joystick input resend
/// - On Screen button + background colour refresh
/// - Gizmo-based radar overlay
pub struct HelmPanelPlugin;

impl Plugin for HelmPanelPlugin {
    fn build(&self, app: &mut App) {
        app
            .insert_resource(HelmJoystickState::default())
            .insert_resource(HelmTickTimer(Timer::from_seconds(0.1, TimerMode::Repeating)))
            .init_resource::<PhoneShipSpeed>()
            .add_systems(Update, (
                spawn_phone_helm_ui
                    .run_if(not(resource_exists::<PhoneHelmSpawned>)),
                toggle_helm_panel_visibility,
                helm_resend_tick,
                refresh_phone_helm_readout,
                update_phone_helm_knob,
                rotate_compass_ring_by_yaw,
                update_radar_readouts,
                handle_on_screen_button_press,
                refresh_on_screen_button_style,
                handle_helm_impulse_button_press,
                draw_helm_radar,
            ));
    }
}

// ── Visibility system ────────────────────────────────────────────────

fn toggle_helm_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<HelmPanel>>,
    mut state: ResMut<HelmJoystickState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    if !lobby.is_changed() && !token.is_changed() && !active.is_changed() {
        return;
    }
    let visible = helm_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
    if !visible && state.active {
        let msg = release(&mut state);
        outbound.write(OutboundClientMessage(msg));
    }
}

// ── 10 Hz resend ────────────────────────────────────────────────────

fn helm_resend_tick(
    time: Res<Time>,
    mut timer: ResMut<HelmTickTimer>,
    state: Res<HelmJoystickState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    timer.0.tick(time.delta());
    if !timer.0.just_finished() {
        return;
    }
    if let Some(msg) = tick(&state) {
        outbound.write(OutboundClientMessage(msg));
    }
}

// ── On Screen button ────────────────────────────────────────────────

fn handle_on_screen_button_press(
    mut interactions: Query<
        &Interaction,
        (Changed<Interaction>, With<Button>, With<OnScreenButton>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(on_screen_message()));
        }
    }
}

fn refresh_on_screen_button_style(
    ship_view: Res<ShipView>,
    mut buttons: Query<&mut BackgroundColor, With<OnScreenButton>>,
) {
    if !ship_view.is_changed() {
        return;
    }
    let color = if matches!(ship_view.view_mode, ViewMode::Radar) {
        ON_SCREEN_BG_ACTIVE
    } else {
        ON_SCREEN_BG_IDLE
    };
    for mut bg in buttons.iter_mut() {
        bg.0 = color;
    }
}

// ── Helm Impulse button ──────────────────────────────────────────────

fn handle_helm_impulse_button_press(
    mut interactions: Query<
        &Interaction,
        (Changed<Interaction>, With<Button>, With<HelmImpulseButton>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(ClientMessage::StartImpulseCharge));
        }
    }
}

// ── Gizmo radar ──────────────────────────────────────────────────────

/// Reads the radar panel's on-screen rect and draws rings, asteroids and
/// ship via `Gizmos` in the Camera2d's world space. Skipped when the
/// helm panel is hidden so we don't paint stale visuals.
fn draw_helm_radar(
    mut gizmos: Gizmos,
    panel: Query<(&ComputedNode, &GlobalTransform, &ViewVisibility), With<RadarPanel>>,
    helm_panel: Query<&Visibility, With<HelmPanel>>,
    sim: Res<ClientSimState>,
    ship_view: Res<ShipView>,
    windows: Query<&Window>,
) {
    if !helm_panel
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

    const ZOOM: f32 = 1.5;
    gizmos.circle_2d(centre, radius, RADAR_OUTER_RING_COLOR);
    let helm_range = crate::client_sim::helm_radar_config().range;
    let mid_ratio = crate::radar::RADAR_MID_RING / helm_range;
    gizmos.circle_2d(centre, radius * mid_ratio / ZOOM, RADAR_MID_RING_COLOR);

    let helm_view = crate::client_sim::compute_helm_radar_view(&sim, &ship_view);
    for dot in &helm_view.dots {
        let pos = centre + Vec2::new(dot.radar_x * radius / ZOOM, dot.radar_y * radius / ZOOM);
        let pix_radius = (dot.scaled_radius * radius / ZOOM).max(2.0);
        gizmos.circle_2d(pos, pix_radius, RADAR_ASTEROID_COLOR);
    }

    let nose_len  = radius * 0.10;
    let half_base = radius * 0.06;
    let nose  = centre + Vec2::new(0.0,  nose_len);
    let left  = centre + Vec2::new(-half_base, -nose_len * 0.6);
    let right = centre + Vec2::new( half_base, -nose_len * 0.6);
    gizmos.line_2d(nose, left,  RADAR_SHIP_COLOR);
    gizmos.line_2d(left, right, RADAR_SHIP_COLOR);
    gizmos.line_2d(right, nose, RADAR_SHIP_COLOR);
}

// ── Phone helm UI spawn ──────────────────────────────────────────────

/// Spawns the phone helm panel (compass-ring radar + polished thumbstick)
/// on the first frame where `PhoneAssets` are available.
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

    let mut pad_entity: Option<Entity> = None;

    commands
        .spawn((
            HelmPanel,
            Node {
                position_type: PositionType::Absolute,
                left:   Val::Px(4.0),
                right:  Val::Px(4.0),
                top:    Val::Px(4.0),
                bottom: Val::Px(4.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexEnd,
                justify_content: JustifyContent::SpaceBetween,
                column_gap: Val::Px(8.0),
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|root| {
            let spawn_joystick = |root: &mut ChildSpawnerCommands, pad_entity: &mut Option<Entity>| {
                root.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|col| {
                    *pad_entity = Some(spawn_helm_joystick_children(col, &assets));
                });
            };
            let spawn_radar = |root: &mut ChildSpawnerCommands| {
                root.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|col| spawn_helm_radar_children(col, &assets));
            };
            if is_landscape {
                spawn_radar(root);
                spawn_joystick(root, &mut pad_entity);
            } else {
                spawn_joystick(root, &mut pad_entity);
                spawn_radar(root);
            }
        });

    if let Some(pad) = pad_entity {
        commands.entity(pad).observe(on_phone_helm_drag_start);
        commands.entity(pad).observe(on_phone_helm_drag);
        commands.entity(pad).observe(on_phone_helm_drag_end);
    }
}

fn spawn_helm_joystick_children(col: &mut ChildSpawnerCommands, assets: &PhoneAssets) -> Entity {
    col.spawn((
        Text::new("▲"),
        TextFont {
            font: assets.font_mono.clone(),
            font_size: 16.0,
            ..default()
        },
        TextColor(Color::srgb(0.55, 0.70, 1.0)),
        Node { ..default() },
    ));

    let mut pad_entity: Option<Entity> = None;

    col.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(4.0),
        ..default()
    })
    .with_children(|row| {
        row.spawn((
            Text::new("◄"),
            TextFont {
                font: assets.font_mono.clone(),
                font_size: 14.0,
                ..default()
            },
            TextColor(Color::srgb(0.55, 0.70, 1.0)),
            Node { ..default() },
        ));

        let pad = row
            .spawn((
                PhoneHelmPad,
                Button,
                Node {
                    width:  Val::Px(HELM_PAD_SIZE),
                    height: Val::Px(HELM_PAD_SIZE),
                    position_type: PositionType::Relative,
                    ..default()
                },
                BackgroundColor(HELM_PAD_BG),
            ))
            .with_children(|pad| {
                pad.spawn((
                    PhoneThumbRing,
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(1.0),
                        top: Val::Px(1.0),
                        width:  Val::Px(HELM_PAD_SIZE - 2.0),
                        height: Val::Px(HELM_PAD_SIZE - 2.0),
                        border: UiRect::all(Val::Px(2.0)),
                        ..default()
                    },
                    BorderColor::all(Color::srgba(0.55, 0.70, 1.0, 0.4)),
                ));
                pad.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(28.0),
                        top: Val::Px(28.0),
                        width:  Val::Px(HELM_PAD_SIZE - 56.0),
                        height: Val::Px(HELM_PAD_SIZE - 56.0),
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BorderColor::all(Color::srgba(0.30, 0.45, 0.70, 0.3)),
                ));
                pad.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(8.0),
                        top:  Val::Px(HELM_PAD_SIZE / 2.0 - 0.5),
                        width:  Val::Px(HELM_PAD_SIZE - 16.0),
                        height: Val::Px(1.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.30, 0.45, 0.70, 0.4)),
                ));
                pad.spawn((
                    Node {
                        position_type: PositionType::Absolute,
                        left: Val::Px(HELM_PAD_SIZE / 2.0 - 0.5),
                        top:  Val::Px(8.0),
                        width:  Val::Px(1.0),
                        height: Val::Px(HELM_PAD_SIZE - 16.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.30, 0.45, 0.70, 0.4)),
                ));
                pad.spawn((
                    PhoneHelmKnob,
                    Node {
                        width:  Val::Px(HELM_KNOB_RADIUS * 2.0),
                        height: Val::Px(HELM_KNOB_RADIUS * 2.0),
                        position_type: PositionType::Absolute,
                        left: Val::Px(HELM_PAD_SIZE / 2.0 - HELM_KNOB_RADIUS),
                        top:  Val::Px(HELM_PAD_SIZE / 2.0 - HELM_KNOB_RADIUS),
                        ..default()
                    },
                    BackgroundColor(HELM_KNOB_BG_IDLE),
                ));
            })
            .id();
        pad_entity = Some(pad);

        row.spawn((
            Text::new("►"),
            TextFont {
                font: assets.font_mono.clone(),
                font_size: 14.0,
                ..default()
            },
            TextColor(Color::srgb(0.55, 0.70, 1.0)),
            Node { ..default() },
        ));
    });

    col.spawn((
        Text::new("▼"),
        TextFont {
            font: assets.font_mono.clone(),
            font_size: 16.0,
            ..default()
        },
        TextColor(Color::srgb(0.55, 0.70, 1.0)),
        Node { ..default() },
    ));

    col.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        column_gap: Val::Px(24.0),
        ..default()
    })
    .with_children(|row| {
        row.spawn((
            Text::new("FWD"),
            TextFont { font: assets.font_mono.clone(), font_size: 9.0, ..default() },
            TextColor(Color::srgb(0.6, 0.7, 0.73)),
        ));
        row.spawn((
            Text::new("REV"),
            TextFont { font: assets.font_mono.clone(), font_size: 9.0, ..default() },
            TextColor(Color::srgb(0.6, 0.7, 0.73)),
        ));
    });

    col.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        column_gap: Val::Px(32.0),
        ..default()
    })
    .with_children(|row| {
        row.spawn((
            Text::new("PORT"),
            TextFont { font: assets.font_mono.clone(), font_size: 9.0, ..default() },
            TextColor(Color::srgb(0.6, 0.7, 0.73)),
        ));
        row.spawn((
            Text::new("STBD"),
            TextFont { font: assets.font_mono.clone(), font_size: 9.0, ..default() },
            TextColor(Color::srgb(0.6, 0.7, 0.73)),
        ));
    });

    col.spawn((
        PhoneHelmReadout,
        Text::new("Thrust 0% / Steering 0%"),
        TextFont { font: assets.font_mono.clone(), font_size: 11.0, ..default() },
        TextColor(Color::srgb(0.6, 0.7, 0.73)),
    ));

    pad_entity.unwrap()
}

fn spawn_helm_radar_children(col: &mut ChildSpawnerCommands, assets: &PhoneAssets) {
    col.spawn((
        PhoneCompassRadar,
        Node {
            width:  Val::Px(COMPASS_RADAR_DIAMETER),
            height: Val::Px(COMPASS_RADAR_DIAMETER),
            position_type: PositionType::Relative,
            ..default()
        },
    ))
    .with_children(|radar| {
        radar.spawn((
            PhoneHdgReadout,
            Text::new("HDG 000°"),
            TextFont { font: assets.font_mono.clone(), font_size: 10.0, ..default() },
            TextColor(Color::srgb(0.55, 0.70, 1.0)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(4.0),
                top:  Val::Px(4.0),
                ..default()
            },
        ));
        radar.spawn((
            PhoneSpdReadout,
            Text::new("SPD --"),
            TextFont { font: assets.font_mono.clone(), font_size: 10.0, ..default() },
            TextColor(Color::srgb(0.55, 0.70, 1.0)),
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(4.0),
                top:   Val::Px(4.0),
                ..default()
            },
        ));
        radar.spawn((
            PhoneXReadout,
            Text::new("X 0"),
            TextFont { font: assets.font_mono.clone(), font_size: 10.0, ..default() },
            TextColor(Color::srgb(0.55, 0.70, 1.0)),
            Node {
                position_type: PositionType::Absolute,
                left:   Val::Px(4.0),
                bottom: Val::Px(4.0),
                ..default()
            },
        ));
        radar.spawn((
            PhoneZReadout,
            Text::new("Z 0"),
            TextFont { font: assets.font_mono.clone(), font_size: 10.0, ..default() },
            TextColor(Color::srgb(0.55, 0.70, 1.0)),
            Node {
                position_type: PositionType::Absolute,
                right:  Val::Px(4.0),
                bottom: Val::Px(4.0),
                ..default()
            },
        ));

        let radar_radius = COMPASS_RADAR_DIAMETER / 2.0;
        let radii = range_ring_radii(radar_radius);
        for &r in &radii {
            let d = r * 2.0;
            let off = radar_radius - r;
            radar.spawn((
                PhoneRangeRing,
                Node {
                    position_type: PositionType::Absolute,
                    left:   Val::Px(off),
                    top:    Val::Px(off),
                    width:  Val::Px(d),
                    height: Val::Px(d),
                    border: UiRect::all(Val::Px(1.0)),
                    ..default()
                },
                BorderColor::all(Color::srgba(0.30, 0.45, 0.70, 0.4)),
            ));
        }

        radar.spawn((
            Node {
                position_type: PositionType::Absolute,
                left:   Val::Px(0.0),
                top:    Val::Px(radar_radius - 0.5),
                width:  Val::Px(COMPASS_RADAR_DIAMETER),
                height: Val::Px(1.0),
                ..default()
            },
            BackgroundColor(Color::srgba(0.20, 0.30, 0.50, 0.3)),
        ));
        radar.spawn((
            Node {
                position_type: PositionType::Absolute,
                left:   Val::Px(radar_radius - 0.5),
                top:    Val::Px(0.0),
                width:  Val::Px(1.0),
                height: Val::Px(COMPASS_RADAR_DIAMETER),
                ..default()
            },
            BackgroundColor(Color::srgba(0.20, 0.30, 0.50, 0.3)),
        ));

        radar.spawn((
            PhoneCompassRing,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(-10.0),
                top:  Val::Px(-10.0),
                width:  Val::Px(COMPASS_RADAR_DIAMETER + 20.0),
                height: Val::Px(COMPASS_RADAR_DIAMETER + 20.0),
                ..default()
            },
        ))
        .with_children(|ring| {
            ring.spawn((
                ImageNode::new(assets.compass_ring.clone()),
                Node {
                    width:  Val::Px(COMPASS_RADAR_DIAMETER + 20.0),
                    height: Val::Px(COMPASS_RADAR_DIAMETER + 20.0),
                    ..default()
                },
            ));
            let centre = (COMPASS_RADAR_DIAMETER + 20.0) / 2.0;
            let tick_outer_r = centre - 4.0;
            for tick in bearing_ticks() {
                let tx = centre + tick_outer_r * tick.angle_rad.sin();
                let ty = centre - tick_outer_r * tick.angle_rad.cos();
                let (tw, th) = if tick.is_major { (2.0, 7.0) } else { (1.0, 4.0) };
                ring.spawn((
                    PhoneCompassTick,
                    Node {
                        position_type: PositionType::Absolute,
                        left:   Val::Px(tx - tw / 2.0),
                        top:    Val::Px(ty - th / 2.0),
                        width:  Val::Px(tw),
                        height: Val::Px(th),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.55, 0.70, 1.0, 0.7)),
                ));
                if tick.is_major {
                    let lr = tick_outer_r - 14.0;
                    let lx = centre + lr * tick.angle_rad.sin();
                    let ly = centre - lr * tick.angle_rad.cos();
                    ring.spawn((
                        Text::new(tick.label.clone()),
                        TextFont { font: assets.font_mono.clone(), font_size: 8.0, ..default() },
                        TextColor(Color::srgba(0.55, 0.70, 1.0, 0.8)),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(lx),
                            top:  Val::Px(ly),
                            ..default()
                        },
                    ));
                }
            }
        });

        radar.spawn((
            Text::new("▲"),
            TextFont { font: assets.font_mono.clone(), font_size: 14.0, ..default() },
            TextColor(Color::srgb(0.95, 0.95, 1.0)),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(radar_radius - 7.0),
                top:  Val::Px(radar_radius - 7.0),
                ..default()
            },
        ));
    });

    col.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        column_gap: Val::Px(8.0),
        ..default()
    })
    .with_children(|row| {
        row.spawn((
            OnScreenButton,
            Button,
            Node {
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.13, 0.13, 0.27)),
        ))
        .with_children(|btn| {
            btn.spawn((
                Text::new("ON SCREEN"),
                TextFont { font: assets.font_display.clone(), font_size: 12.0, ..default() },
                TextColor(Color::srgb(0.93, 0.93, 1.0)),
            ));
        });
        row.spawn((
            HelmImpulseButton,
            Button,
            Node {
                padding: UiRect::axes(Val::Px(12.0), Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(0.10, 0.25, 0.40)),
        ))
        .with_children(|btn| {
            btn.spawn((
                Text::new("IMPULSE"),
                TextFont { font: assets.font_display.clone(), font_size: 12.0, ..default() },
                TextColor(Color::srgb(0.5, 0.8, 1.0)),
            ));
        });
    });
}

// ── Pointer observers ────────────────────────────────────────────────

fn on_phone_helm_drag_start(
    trigger: On<Pointer<DragStart>>,
    mut state: ResMut<HelmJoystickState>,
    mut knob_bg: Query<&mut BackgroundColor, With<PhoneHelmKnob>>,
) {
    let _ = trigger;
    state.active = true;
    for mut bg in knob_bg.iter_mut() {
        bg.0 = HELM_KNOB_BG_ACTIVE;
    }
}

fn on_phone_helm_drag(
    trigger: On<Pointer<Drag>>,
    mut state: ResMut<HelmJoystickState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    let drag_event = trigger.event();
    let new_dx = state.knob_dx + drag_event.delta.x;
    let new_dy = state.knob_dy + drag_event.delta.y;
    if let Some(msg) = drag(&mut state, new_dx, new_dy, helm_max_radius()) {
        outbound.write(OutboundClientMessage(msg));
    }
}

fn on_phone_helm_drag_end(
    trigger: On<Pointer<DragEnd>>,
    mut state: ResMut<HelmJoystickState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
    mut knob_bg: Query<&mut BackgroundColor, With<PhoneHelmKnob>>,
) {
    let _ = trigger;
    let msg = release(&mut state);
    outbound.write(OutboundClientMessage(msg));
    for mut bg in knob_bg.iter_mut() {
        bg.0 = HELM_KNOB_BG_IDLE;
    }
}

// ── Update systems ───────────────────────────────────────────────────

fn rotate_compass_ring_by_yaw(
    ship_view: Res<ShipView>,
    mut rings: Query<&mut Transform, With<PhoneCompassRing>>,
) {
    if !ship_view.is_changed() {
        return;
    }
    for mut tf in rings.iter_mut() {
        tf.rotation = Quat::from_rotation_z(ship_view.ship_yaw);
    }
}

fn update_phone_helm_knob(
    state: Res<HelmJoystickState>,
    mut knobs: Query<&mut Node, With<PhoneHelmKnob>>,
) {
    if !state.is_changed() {
        return;
    }
    let centre = HELM_PAD_SIZE / 2.0 - HELM_KNOB_RADIUS;
    for mut node in knobs.iter_mut() {
        node.left = Val::Px(centre + state.knob_dx);
        node.top  = Val::Px(centre + state.knob_dy);
    }
}

fn refresh_phone_helm_readout(
    state: Res<HelmJoystickState>,
    mut readouts: Query<&mut Text, With<PhoneHelmReadout>>,
) {
    if !state.is_changed() {
        return;
    }
    let thrust_pct   = (state.last_thrust   * 100.0).round() as i32;
    let steering_pct = (state.last_steering * 100.0).round() as i32;
    for mut text in readouts.iter_mut() {
        **text = format!("Thrust {thrust_pct}% / Steering {steering_pct}%");
    }
}

fn update_radar_readouts(
    ship_view: Res<ShipView>,
    mut speed: ResMut<PhoneShipSpeed>,
    mut hdg: Query<&mut Text, (With<PhoneHdgReadout>, Without<PhoneSpdReadout>, Without<PhoneXReadout>, Without<PhoneZReadout>)>,
    mut spd: Query<&mut Text, (With<PhoneSpdReadout>, Without<PhoneHdgReadout>, Without<PhoneXReadout>, Without<PhoneZReadout>)>,
    mut x_read: Query<&mut Text, (With<PhoneXReadout>, Without<PhoneHdgReadout>, Without<PhoneSpdReadout>, Without<PhoneZReadout>)>,
    mut z_read: Query<&mut Text, (With<PhoneZReadout>, Without<PhoneHdgReadout>, Without<PhoneSpdReadout>, Without<PhoneXReadout>)>,
) {
    if !ship_view.is_changed() {
        return;
    }

    if speed.initialized {
        let dx = ship_view.ship_x - speed.prev_x;
        let dz = ship_view.ship_z - speed.prev_z;
        speed.speed = (dx * dx + dz * dz).sqrt();
    } else {
        speed.initialized = true;
    }
    speed.prev_x = ship_view.ship_x;
    speed.prev_z = ship_view.ship_z;

    for mut text in hdg.iter_mut() {
        **text = format!("HDG {}", yaw_to_heading(ship_view.ship_yaw));
    }
    for mut text in spd.iter_mut() {
        **text = format!("SPD {:.0}", speed.speed);
    }
    for mut text in x_read.iter_mut() {
        **text = format!("X {:.0}", ship_view.ship_x);
    }
    for mut text in z_read.iter_mut() {
        **text = format!("Z {:.0}", ship_view.ship_z);
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client_lobby::{ActiveConsole, LobbyState};
    use crate::messages::{GamePhase, Console, GameState, Player, ServerMessage};
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

    // ── bearing_ticks ─────────────────────────────────────────────

    #[test]
    fn bearing_ticks_returns_36_ticks() {
        let ticks = bearing_ticks();
        assert_eq!(ticks.len(), 36);
    }

    #[test]
    fn bearing_ticks_first_tick_is_zero() {
        let ticks = bearing_ticks();
        assert_eq!(ticks[0].angle_deg, 0.0);
        assert_eq!(ticks[0].angle_rad, 0.0);
        assert!(ticks[0].is_major);
        assert_eq!(ticks[0].label, "00");
    }

    #[test]
    fn bearing_ticks_every_third_is_major_with_label() {
        let ticks = bearing_ticks();
        for (i, tick) in ticks.iter().enumerate() {
            if i % 3 == 0 {
                assert!(tick.is_major, "tick {i} should be major");
                assert!(!tick.label.is_empty(), "tick {i} label should not be empty");
            } else {
                assert!(!tick.is_major, "tick {i} should NOT be major");
                assert!(tick.label.is_empty(), "tick {i} label should be empty");
            }
        }
    }

    #[test]
    fn bearing_ticks_labels_are_sequential_10s() {
        let ticks = bearing_ticks();
        for (i, tick) in ticks.iter().enumerate().step_by(3) {
            let expected_deg = i as f32 * 10.0;
            assert_eq!(tick.angle_deg, expected_deg);
            assert_eq!(tick.label, format!("{:02}", expected_deg as u32));
        }
    }

    #[test]
    fn bearing_ticks_last_label_is_330() {
        let ticks = bearing_ticks();
        assert_eq!(ticks[33].label, "330");
        assert_eq!(ticks[33].angle_deg, 330.0);
    }

    #[test]
    fn bearing_ticks_minor_tick_has_empty_label() {
        let ticks = bearing_ticks();
        assert!(ticks[1].label.is_empty());
        assert!(ticks[2].label.is_empty());
        assert!(!ticks[3].label.is_empty());
    }

    // ── range_ring_radii ───────────────────────────────────────────

    #[test]
    fn range_ring_radii_returns_three_values() {
        let radii = range_ring_radii(300.0);
        assert_eq!(radii.len(), 3);
    }

    #[test]
    fn range_ring_radii_have_proportional_spacing() {
        let radii = range_ring_radii(300.0);
        let close = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(close(radii[0], 100.0), "first ring should be 1/3 radius, got {}", radii[0]);
        assert!(close(radii[1], 200.0), "second ring should be 2/3 radius, got {}", radii[1]);
        assert!(close(radii[2], 300.0), "third ring should be full radius, got {}", radii[2]);
    }

    #[test]
    fn range_ring_radii_scales_with_input() {
        let radii = range_ring_radii(150.0);
        let close = |a: f32, b: f32| (a - b).abs() < 0.01;
        assert!(close(radii[0], 50.0));
        assert!(close(radii[1], 100.0));
        assert!(close(radii[2], 150.0));
    }

    // ── range_ring_labels ──────────────────────────────────────────

    #[test]
    fn range_ring_labels_are_strs() {
        let labels = range_ring_labels();
        assert_eq!(labels.len(), 3);
        assert_eq!(labels[0], "200");
        assert_eq!(labels[1], "400");
        assert_eq!(labels[2], "600");
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
