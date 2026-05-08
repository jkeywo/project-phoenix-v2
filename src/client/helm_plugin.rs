use bevy::prelude::*;

use crate::client::app::OutboundClientMessage;
use crate::client::lobby_state::{LobbyState, LobbyView, LocalPlayerToken};
use crate::client::sim_state::{on_screen_message, ClientSimState};
use crate::client::helm_state::{drag, press, release, tick, HelmJoystickState};
use crate::shared::messages::GamePhase;
use crate::shared::radar;

// ── Marker components ──────────────────────────────────────────────

/// Marks the root of the helm joystick UI; shown only when the local
/// player holds Helm and the phase is InProgress.
#[derive(Component)]
pub struct HelmPanel;

/// Marks the circular pad that captures pointer drag events.
#[derive(Component)]
pub struct HelmPad;

/// Marks the small movable knob nested inside the pad.
#[derive(Component)]
pub struct HelmKnob;

/// Marks the text node showing live "Thrust X% / Steering Y%" values.
#[derive(Component)]
pub struct HelmReadout;

/// Marks the radar panel container. Its `ComputedNode` size and on-screen
/// position drive the gizmos that draw the radar visuals.
#[derive(Component)]
pub struct RadarPanel;

/// Marks the "On Screen" button on the helm console; pressing it sends
/// `SetView { mode: Radar }` so the server viewscreen mirrors the radar.
#[derive(Component)]
pub struct OnScreenButton;

// ── Resources ──────────────────────────────────────────────────────

/// 10Hz resend timer for the helm joystick.
#[derive(Resource)]
pub struct HelmTickTimer(pub Timer);

// ── Constants ──────────────────────────────────────────────────────

/// Diameter of the joystick pad in logical pixels. The knob is constrained
/// to a circle whose radius is `(PAD_SIZE / 2) - HELM_KNOB_RADIUS - 2`,
/// matching the JS contract.
pub const HELM_PAD_SIZE: f32 = 200.0;
/// Radius of the knob disc, in pixels.
pub const HELM_KNOB_RADIUS: f32 = 24.0;
/// Background colour of the joystick pad.
pub const HELM_PAD_BG: Color = Color::srgb(0.10, 0.10, 0.18);
/// Knob colour while idle.
pub const HELM_KNOB_BG_IDLE: Color = Color::srgb(0.27, 0.27, 0.40);
/// Knob colour while being dragged.
pub const HELM_KNOB_BG_ACTIVE: Color = Color::srgb(0.40, 0.40, 0.67);

/// Outer ring colour for the helm radar.
pub const RADAR_OUTER_RING_COLOR: Color = Color::srgb(0.55, 0.70, 1.0);
/// Mid ring colour (drawn at `RADAR_MID_RING / RADAR_RANGE` of the outer
/// radius).
pub const RADAR_MID_RING_COLOR:   Color = Color::srgb(0.30, 0.40, 0.65);
/// Asteroid blip colour.
pub const RADAR_ASTEROID_COLOR:   Color = Color::srgb(0.85, 0.75, 0.45);
/// Ship triangle colour (always points "up" since the radar is
/// ship-aligned).
pub const RADAR_SHIP_COLOR:       Color = Color::srgb(0.95, 0.95, 1.0);

/// Effective max drag radius, derived from `HELM_PAD_SIZE` and
/// `HELM_KNOB_RADIUS` exactly the way the JS code did. Centralised so
/// pad/knob/clamp logic agree.
pub fn helm_max_radius() -> f32 {
    (HELM_PAD_SIZE / 2.0) - HELM_KNOB_RADIUS - 2.0
}

// ── Plugin ─────────────────────────────────────────────────────────

pub struct HelmConsolePlugin;

impl Plugin for HelmConsolePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(HelmJoystickState::default())
            .insert_resource(HelmTickTimer(Timer::from_seconds(0.1, TimerMode::Repeating)))
            .add_systems(Startup, setup_helm_ui)
            .add_systems(Update, (
                toggle_helm_panel_visibility,
                helm_resend_tick,
                refresh_helm_knob_position,
                refresh_helm_readout,
                handle_on_screen_button_press,
                draw_helm_radar,
            ));
    }
}

// ── Setup ──────────────────────────────────────────────────────────

fn setup_helm_ui(mut commands: Commands) {
    let mut pad_entity: Option<Entity> = None;

    commands
        .spawn((
            HelmPanel,
            Node {
                position_type: PositionType::Absolute,
                left:   Val::Px(16.0),
                bottom: Val::Px(16.0),
                right:  Val::Px(16.0),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexEnd,
                justify_content: JustifyContent::SpaceBetween,
                column_gap: Val::Px(16.0),
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            // ── Left column: joystick + readout ─────────────────────
            panel
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::FlexStart,
                        row_gap: Val::Px(8.0),
                        ..default()
                    },
                ))
                .with_children(|col| {
                    col.spawn((
                        HelmReadout,
                        Text::new("Thrust 0% / Steering 0%"),
                        TextFont { font_size: 14.0, ..default() },
                        TextColor(Color::srgb(0.6, 0.7, 0.73)),
                    ));

                    let pad = col
                        .spawn((
                            HelmPad,
                            // `Button` opts the node into the picking
                            // backend so observers fire reliably on drag
                            // start/move/end.
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
                                HelmKnob,
                                Node {
                                    width:  Val::Px(HELM_KNOB_RADIUS * 2.0),
                                    height: Val::Px(HELM_KNOB_RADIUS * 2.0),
                                    position_type: PositionType::Absolute,
                                    // Centre the knob: anchor by top-left
                                    // then offset by -radius so
                                    // (left,top) == centre.
                                    left: Val::Px(HELM_PAD_SIZE / 2.0 - HELM_KNOB_RADIUS),
                                    top:  Val::Px(HELM_PAD_SIZE / 2.0 - HELM_KNOB_RADIUS),
                                    ..default()
                                },
                                BackgroundColor(HELM_KNOB_BG_IDLE),
                            ));
                        })
                        .id();
                    pad_entity = Some(pad);
                });

            // ── Right column: radar + On Screen button ──────────────
            panel
                .spawn((
                    Node {
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Center,
                        row_gap: Val::Px(8.0),
                        ..default()
                    },
                ))
                .with_children(|col| {
                    col.spawn((
                        RadarPanel,
                        Node {
                            // 90% of the narrowest viewport dimension,
                            // matching the JS contract from the PRD.
                            width:  Val::VMin(90.0),
                            height: Val::VMin(90.0),
                            // Cap radar at twice the joystick pad so it
                            // doesn't dominate large landscape windows.
                            max_width:  Val::Px(HELM_PAD_SIZE * 2.0),
                            max_height: Val::Px(HELM_PAD_SIZE * 2.0),
                            ..default()
                        },
                        // Transparent — the gizmos overlay does the
                        // drawing. A faint border helps with debug
                        // alignment but otherwise is invisible.
                        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.20)),
                    ));

                    col.spawn((
                        OnScreenButton,
                        Button,
                        Node {
                            padding: UiRect::all(Val::Px(10.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.13, 0.13, 0.27)),
                    ))
                    .with_children(|btn| {
                        btn.spawn((
                            Text::new("On Screen"),
                            TextFont { font_size: 16.0, ..default() },
                            TextColor(Color::srgb(0.93, 0.93, 1.0)),
                        ));
                    });
                });
        });

    // Pointer-event observers on the pad. Each handler updates the shared
    // `HelmJoystickState` resource and emits an `OutboundClientMessage`.
    if let Some(pad) = pad_entity {
        commands.entity(pad).observe(on_helm_drag_start);
        commands.entity(pad).observe(on_helm_drag);
        commands.entity(pad).observe(on_helm_drag_end);
    }
}

// ── Systems ────────────────────────────────────────────────────────

fn toggle_helm_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    mut panel: Query<&mut Visibility, With<HelmPanel>>,
    mut state: ResMut<HelmJoystickState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    if !lobby.is_changed() && !token.is_changed() {
        return;
    }
    let view = LobbyView::new(&lobby, &token.0);
    let visible = lobby.phase == GamePhase::InProgress && view.is_helm();
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
    // If the helm panel disappears mid-drag (e.g. console released), make
    // sure the ship stops by emitting a final zero `HelmInput`.
    if !visible && state.active {
        let msg = release(&mut state);
        outbound.write(OutboundClientMessage(msg));
    }
}

fn on_helm_drag_start(
    trigger: On<Pointer<DragStart>>,
    mut state: ResMut<HelmJoystickState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
    mut knob_bg: Query<&mut BackgroundColor, With<HelmKnob>>,
) {
    // We don't care about which entity triggered — there's only one pad.
    let _ = trigger;
    let msg = press(&mut state, 0.0, 0.0, helm_max_radius());
    outbound.write(OutboundClientMessage(msg));
    for mut bg in knob_bg.iter_mut() {
        bg.0 = HELM_KNOB_BG_ACTIVE;
    }
}

fn on_helm_drag(
    trigger: On<Pointer<Drag>>,
    mut state: ResMut<HelmJoystickState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    let drag_event = trigger.event();
    if let Some(msg) = drag(
        &mut state,
        drag_event.distance.x,
        drag_event.distance.y,
        helm_max_radius(),
    ) {
        outbound.write(OutboundClientMessage(msg));
    }
}

fn on_helm_drag_end(
    trigger: On<Pointer<DragEnd>>,
    mut state: ResMut<HelmJoystickState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
    mut knob_bg: Query<&mut BackgroundColor, With<HelmKnob>>,
) {
    let _ = trigger;
    let msg = release(&mut state);
    outbound.write(OutboundClientMessage(msg));
    for mut bg in knob_bg.iter_mut() {
        bg.0 = HELM_KNOB_BG_IDLE;
    }
}

/// 10Hz repeating timer: while the joystick is active, resend the most
/// recent `HelmInput` so the server keeps applying it.
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

fn refresh_helm_knob_position(
    state: Res<HelmJoystickState>,
    mut knob: Query<&mut Node, With<HelmKnob>>,
) {
    if !state.is_changed() {
        return;
    }
    let centre = HELM_PAD_SIZE / 2.0 - HELM_KNOB_RADIUS;
    for mut node in knob.iter_mut() {
        node.left = Val::Px(centre + state.knob_dx);
        node.top  = Val::Px(centre + state.knob_dy);
    }
}

fn refresh_helm_readout(
    state: Res<HelmJoystickState>,
    mut readout: Query<&mut Text, With<HelmReadout>>,
) {
    if !state.is_changed() {
        return;
    }
    let thrust_pct   = (state.last_thrust   * 100.0).round() as i32;
    let steering_pct = (state.last_steering * 100.0).round() as i32;
    for mut text in readout.iter_mut() {
        **text = format!("Thrust {thrust_pct}% / Steering {steering_pct}%");
    }
}

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

/// Reads the radar panel's on-screen rect and draws rings, asteroids and
/// ship via `Gizmos` in the Camera2d's world space. Skipped when the
/// helm panel is hidden so we don't paint stale visuals.
fn draw_helm_radar(
    mut gizmos: Gizmos,
    panel: Query<(&ComputedNode, &GlobalTransform, &ViewVisibility), With<RadarPanel>>,
    helm_panel: Query<&Visibility, With<HelmPanel>>,
    sim: Res<ClientSimState>,
    windows: Query<&Window>,
) {
    // Only draw while the helm panel is shown.
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

    // UI node positions are in logical screen pixels with origin at the
    // top-left and +y down; the Camera2d default is centred on origin
    // with +y up. Convert centre + size accordingly.
    let node_centre_screen = gt.translation().truncate();
    let centre_world_x = node_centre_screen.x - viewport_w / 2.0;
    let centre_world_y = viewport_h / 2.0 - node_centre_screen.y;
    let centre = Vec2::new(centre_world_x, centre_world_y);

    let size = node.size();
    let radius = size.x.min(size.y) * 0.5;
    if radius <= 0.0 {
        return;
    }

    // Outer + mid rings.
    gizmos.circle_2d(centre, radius, RADAR_OUTER_RING_COLOR);
    let mid_ratio = radar::RADAR_MID_RING / radar::RADAR_RANGE;
    gizmos.circle_2d(centre, radius * mid_ratio, RADAR_MID_RING_COLOR);

    // Asteroids.
    for (rx, ry, rr) in radar::radar_dots(&sim.world.asteroids, sim.ship_x, sim.ship_z, sim.ship_yaw) {
        let pos = centre + Vec2::new(rx * radius, ry * radius);
        let pix_radius = (rr * radius).max(2.0);
        gizmos.circle_2d(pos, pix_radius, RADAR_ASTEROID_COLOR);
    }

    // Ship triangle, always pointing "up" (radar is ship-aligned).
    let nose_len  = radius * 0.10;
    let half_base = radius * 0.06;
    let nose  = centre + Vec2::new(0.0,  nose_len);
    let left  = centre + Vec2::new(-half_base, -nose_len * 0.6);
    let right = centre + Vec2::new( half_base, -nose_len * 0.6);
    gizmos.line_2d(nose, left,  RADAR_SHIP_COLOR);
    gizmos.line_2d(left, right, RADAR_SHIP_COLOR);
    gizmos.line_2d(right, nose, RADAR_SHIP_COLOR);
}
