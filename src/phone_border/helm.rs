//! Phone helm panel — compass-ring radar + polished thumbstick.
//!
//! Replaces the simple gizmo-based helm radar with a full compass-ring
//! display and a polished thumbstick with concentric rings, cross-hairs,
//! directional arrows, and axis labels.
//!
//! The knob and pointer-drag behaviour reuse `client_helm::{drag, release,
//! tick}` unchanged.
//!
//! This module is compiled when the `client` cargo feature is active.

use bevy::prelude::*;

use crate::client_app::{
    HelmPanel, OnScreenButton,
    OutboundClientMessage,
};
use crate::client_helm::{HelmJoystickState, drag, release};
use crate::client_sim::ClientSimState;
use crate::phone_border::framing::PhoneAssets;

// ── Constants ────────────────────────────────────────────────────────

/// Diameter of the compass-ring radar in logical pixels.
pub const COMPASS_RADAR_DIAMETER: f32 = 280.0;

/// World-space distances labelled on each range ring (from closest to
/// farthest).
pub const RANGE_RING_LABEL_DISTANCES: [f32; 3] = [200.0, 400.0, 600.0];

/// Diameter of the joystick pad in logical pixels.
pub const HELM_PAD_SIZE: f32 = 200.0;

/// Radius of the knob disc, in pixels.
pub const HELM_KNOB_RADIUS: f32 = 24.0;

/// Background colour of the joystick pad.
pub const HELM_PAD_BG: Color = Color::srgb(0.10, 0.10, 0.18);

/// Knob colour while idle.
pub const HELM_KNOB_BG_IDLE: Color = Color::srgb(0.27, 0.27, 0.40);

/// Knob colour while being dragged.
pub const HELM_KNOB_BG_ACTIVE: Color = Color::srgb(0.40, 0.40, 0.67);

/// Effective max drag radius.
pub fn helm_max_radius() -> f32 {
    (HELM_PAD_SIZE / 2.0) - HELM_KNOB_RADIUS - 2.0
}

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

// ── Marker components (phone-border helm) ────────────────────────────

/// Marks the outermost container of the compass-ring radar.
#[derive(Component)]
pub struct PhoneCompassRadar;

/// Marks the rotating ring container whose `Transform` rotation is driven
/// by the ship's yaw.
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

/// Marks the pad entity that captures pointer drag events (phone variant).
#[derive(Component)]
pub struct PhoneHelmPad;

/// Marks the knob entity inside the phone thumbstick.
#[derive(Component)]
pub struct PhoneHelmKnob;

/// Marks the phone variant of the thrust/steering readout.
#[derive(Component)]
pub struct PhoneHelmReadout;

// ── Plugin ───────────────────────────────────────────────────────────

/// Replaces the simple gizmo radar + plain thumbstick with a full
/// compass-ring radar display and a polished thumbstick.
pub struct HelmPanelPlugin;

impl Plugin for HelmPanelPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PhoneShipSpeed>()
            .add_systems(Update, (
                spawn_phone_helm_ui
                    .run_if(not(resource_exists::<PhoneHelmSpawned>)),
                rotate_compass_ring_by_yaw,
                update_phone_helm_knob,
                refresh_phone_helm_readout,
                update_radar_readouts,
            ));
    }
}

/// Marker resource set once the phone helm UI has been spawned.
#[derive(Resource)]
pub struct PhoneHelmSpawned;

/// Tracks the ship's computed speed from position deltas.
#[derive(Resource, Default)]
pub struct PhoneShipSpeed {
    pub speed: f32,
    prev_x: f32,
    prev_z: f32,
    initialized: bool,
}

// ── One-shot init system ─────────────────────────────────────────────

/// Spawns the phone helm panel (compass-ring radar + polished thumbstick)
/// on the first frame where `PhoneAssets` are available.
fn spawn_phone_helm_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    old_panel: Query<Entity, With<HelmPanel>>,
) {
    let Some(assets) = assets else { return };

    // Despawn any stale helm panel from a previous setup.
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
            // ── Left column: polished thumbstick ───────────────────
            root
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|col| {
                    // Up arrow (FWD)
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

                    // Row: ◄ arrow + pad + ► arrow
                    col
                        .spawn(Node {
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
                                    // Outer ring
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

                                    // Mid ring
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

                                    // Horizontal cross-hair
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
                                    // Vertical cross-hair
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

                                    // Knob
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

                    // Down arrow (REV)
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

                    // FWD / REV axis labels
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
                            TextFont {
                                font: assets.font_mono.clone(),
                                font_size: 9.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.6, 0.7, 0.73)),
                        ));
                        row.spawn((
                            Text::new("REV"),
                            TextFont {
                                font: assets.font_mono.clone(),
                                font_size: 9.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.6, 0.7, 0.73)),
                        ));
                    });

                    // PORT / STBD axis labels
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
                            TextFont {
                                font: assets.font_mono.clone(),
                                font_size: 9.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.6, 0.7, 0.73)),
                        ));
                        row.spawn((
                            Text::new("STBD"),
                            TextFont {
                                font: assets.font_mono.clone(),
                                font_size: 9.0,
                                ..default()
                            },
                            TextColor(Color::srgb(0.6, 0.7, 0.73)),
                        ));
                    });

                    // Thrust/steering readout
                    col.spawn((
                        PhoneHelmReadout,
                        Text::new("Thrust 0% / Steering 0%"),
                        TextFont {
                            font: assets.font_mono.clone(),
                            font_size: 11.0,
                            ..default()
                        },
                        TextColor(Color::srgb(0.6, 0.7, 0.73)),
                    ));
                });

            // ── Right column: compass-ring radar + buttons ────────
            root
                .spawn(Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(6.0),
                    ..default()
                })
                .with_children(|col| {
                    // Radar frame
                    col
                        .spawn((
                            PhoneCompassRadar,
                            Node {
                                width:  Val::Px(COMPASS_RADAR_DIAMETER),
                                height: Val::Px(COMPASS_RADAR_DIAMETER),
                                position_type: PositionType::Relative,
                                ..default()
                            },
                        ))
                        .with_children(|radar| {
                            // ── Corner readouts ────────────────────
                            radar.spawn((
                                PhoneHdgReadout,
                                Text::new("HDG 000°"),
                                TextFont {
                                    font: assets.font_mono.clone(),
                                    font_size: 10.0,
                                    ..default()
                                },
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
                                TextFont {
                                    font: assets.font_mono.clone(),
                                    font_size: 10.0,
                                    ..default()
                                },
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
                                TextFont {
                                    font: assets.font_mono.clone(),
                                    font_size: 10.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.55, 0.70, 1.0)),
                                Node {
                                    position_type: PositionType::Absolute,
                                    left:  Val::Px(4.0),
                                    bottom: Val::Px(4.0),
                                    ..default()
                                },
                            ));
                            radar.spawn((
                                PhoneZReadout,
                                Text::new("Z 0"),
                                TextFont {
                                    font: assets.font_mono.clone(),
                                    font_size: 10.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.55, 0.70, 1.0)),
                                Node {
                                    position_type: PositionType::Absolute,
                                    right:  Val::Px(4.0),
                                    bottom: Val::Px(4.0),
                                    ..default()
                                },
                            ));

                            // ── Range rings (non-rotating) ──────────
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

                            // ── Cross-hair lines ───────────────────
                            radar.spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left:  Val::Px(0.0),
                                    top:   Val::Px(radar_radius - 0.5),
                                    width:  Val::Px(COMPASS_RADAR_DIAMETER),
                                    height: Val::Px(1.0),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.20, 0.30, 0.50, 0.3)),
                            ));
                            radar.spawn((
                                Node {
                                    position_type: PositionType::Absolute,
                                    left:  Val::Px(radar_radius - 0.5),
                                    top:   Val::Px(0.0),
                                    width:  Val::Px(1.0),
                                    height: Val::Px(COMPASS_RADAR_DIAMETER),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.20, 0.30, 0.50, 0.3)),
                            ));

                            // ── Compass ring (rotating container) ──
                            radar
                                .spawn((
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
                                    // Ring image
                                    ring.spawn((
                                        ImageNode::new(assets.compass_ring.clone()),
                                        Node {
                                            width:  Val::Px(COMPASS_RADAR_DIAMETER + 20.0),
                                            height: Val::Px(COMPASS_RADAR_DIAMETER + 20.0),
                                            ..default()
                                        },
                                    ));

                                    // Bearing ticks
                                    let centre = (COMPASS_RADAR_DIAMETER + 20.0) / 2.0;
                                    let tick_outer_r = centre - 4.0;
                                    for tick in bearing_ticks() {
                                        let tx = centre + tick_outer_r * tick.angle_rad.sin();
                                        let ty = centre - tick_outer_r * tick.angle_rad.cos();
                                        let (tw, th) = if tick.is_major {
                                            (2.0, 7.0)
                                        } else {
                                            (1.0, 4.0)
                                        };
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
                                                TextFont {
                                                    font: assets.font_mono.clone(),
                                                    font_size: 8.0,
                                                    ..default()
                                                },
                                                TextColor(Color::srgba(0.55, 0.70, 1.0, 0.8)),
                                                Node {
                                                    position_type: PositionType::Absolute,
                                                    left:   Val::Px(lx),
                                                    top:    Val::Px(ly),
                                                    ..default()
                                                },
                                            ));
                                        }
                                    }
                                });

                            // ── Ship triangle at centre ────────────
                            radar.spawn((
                                Text::new("▲"),
                                TextFont {
                                    font: assets.font_mono.clone(),
                                    font_size: 14.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.95, 0.95, 1.0)),
                                Node {
                                    position_type: PositionType::Absolute,
                                    left:   Val::Px(radar_radius - 7.0),
                                    top:    Val::Px(radar_radius - 7.0),
                                    ..default()
                                },
                            ));
                        });

                    // ── On Screen + Repair buttons ────────────────
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
                                TextFont {
                                    font: assets.font_display.clone(),
                                    font_size: 12.0,
                                    ..default()
                                },
                                TextColor(Color::srgb(0.93, 0.93, 1.0)),
                            ));
                        });
                    });
                });
        });

    // Register pointer-event observers on the thumbstick pad.
    if let Some(pad) = pad_entity {
        commands.entity(pad).observe(on_phone_helm_drag_start);
        commands.entity(pad).observe(on_phone_helm_drag);
        commands.entity(pad).observe(on_phone_helm_drag_end);
    }
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

/// Rotate the compass ring container so that the bearing at the top
/// matches the ship's current heading (yaw).
fn rotate_compass_ring_by_yaw(
    sim: Res<ClientSimState>,
    mut rings: Query<&mut Transform, With<PhoneCompassRing>>,
) {
    if !sim.is_changed() {
        return;
    }
    for mut tf in rings.iter_mut() {
        tf.rotation = Quat::from_rotation_z(sim.ship_yaw);
    }
}

/// Move the thumbstick knob to match `HelmJoystickState` position.
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

/// Update the thrust/steering readout text below the thumbstick.
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

/// Update the HDG/SPD/X/Z readout text nodes in the four radar corners.
fn update_radar_readouts(
    sim: Res<ClientSimState>,
    mut speed: ResMut<PhoneShipSpeed>,
    mut hdg: Query<&mut Text, (With<PhoneHdgReadout>, Without<PhoneSpdReadout>, Without<PhoneXReadout>, Without<PhoneZReadout>)>,
    mut spd: Query<&mut Text, (With<PhoneSpdReadout>, Without<PhoneHdgReadout>, Without<PhoneXReadout>, Without<PhoneZReadout>)>,
    mut x_read: Query<&mut Text, (With<PhoneXReadout>, Without<PhoneHdgReadout>, Without<PhoneSpdReadout>, Without<PhoneZReadout>)>,
    mut z_read: Query<&mut Text, (With<PhoneZReadout>, Without<PhoneHdgReadout>, Without<PhoneSpdReadout>, Without<PhoneXReadout>)>,
) {
    if !sim.is_changed() {
        return;
    }

    // Compute speed from position delta.
    if speed.initialized {
        let dx = sim.ship_x - speed.prev_x;
        let dz = sim.ship_z - speed.prev_z;
        speed.speed = (dx * dx + dz * dz).sqrt();
    } else {
        speed.initialized = true;
    }
    speed.prev_x = sim.ship_x;
    speed.prev_z = sim.ship_z;

    // HDG from yaw
    for mut text in hdg.iter_mut() {
        **text = format!("HDG {}", yaw_to_heading(sim.ship_yaw));
    }

    // SPD
    for mut text in spd.iter_mut() {
        **text = format!("SPD {:.0}", speed.speed);
    }

    // X coordinate
    for mut text in x_read.iter_mut() {
        **text = format!("X {:.0}", sim.ship_x);
    }

    // Z coordinate
    for mut text in z_read.iter_mut() {
        **text = format!("Z {:.0}", sim.ship_z);
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::{FRAC_PI_4, TAU};

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
