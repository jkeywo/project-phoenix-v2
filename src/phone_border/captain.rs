use bevy::prelude::*;

use crate::client_app::{CaptainPanel, OutboundClientMessage};
use crate::client_sim::ClientSimState;
use crate::messages::{ClientMessage, ViewDirection, ViewMode};
use crate::phone_border::framing::PhoneAssets;

// ── Constants ──

/// Diameter of the compass dial in logical pixels.
const COMPASS_DIAMETER: f32 = 240.0;

/// Size of each direction pad button.
const PAD_BTN_SIZE: f32 = 52.0;

/// Size of the LED indicator dot.
const LED_SIZE: f32 = 8.0;

/// Size of the needle image.
const NEEDLE_SIZE: f32 = 64.0;

/// Gap between the compass and the red alert toggle.
const COMPASS_TO_ALERT_GAP: f32 = 24.0;

// ── Colours ──

const DIR_BG_IDLE: Color = Color::srgb(0.12, 0.12, 0.26);
const DIR_BG_ACTIVE: Color = Color::srgb(0.20, 0.28, 0.50);
const DIR_BORDER: Color = Color::srgba(0.55, 0.70, 1.0, 0.25);
const LED_OFF: Color = Color::srgb(0.12, 0.12, 0.22);
const LED_ON: Color = Color::srgb(0.2, 0.9, 0.2);
const GLYPH_COLOR: Color = Color::srgb(0.90, 0.90, 1.0);
const LABEL_COLOR: Color = Color::srgb(0.55, 0.65, 0.70);
const RA_BG_IDLE: Color = Color::srgb(0.13, 0.13, 0.27);
const RA_BG_ACTIVE: Color = Color::srgb(0.40, 0.0, 0.0);

// ── Marker components ──

/// Marks a direction pad button with its target view direction.
#[derive(Component)]
pub struct DirButton(pub ViewDirection);

/// Marks the LED indicator inside a direction pad button.
#[derive(Component)]
pub struct DirLed;

/// Marks the compass dial root.
#[derive(Component)]
pub struct CompassDial;

/// Marks the rotating compass needle.
#[derive(Component)]
pub struct CompassNeedle;

/// Marks the red alert toggle button.
#[derive(Component)]
pub struct RedAlertToggle;

/// Marks the armed glow indicator on the red alert button.
#[derive(Component)]
pub struct ArmedGlow;

/// One-shot resource set after the captain UI is first spawned.
#[derive(Resource)]
pub struct CaptainPanelSpawned;

// ── Plugin ──

/// Replaces the simple grid-based captain UI with a direction pad overlaid on
/// a compass ring, a rotating needle, and a red alert toggle with armed glow.
pub struct CaptainPanelPlugin;

impl Plugin for CaptainPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (
            spawn_captain_ui.run_if(not(resource_exists::<CaptainPanelSpawned>)),
            refresh_dir_highlights,
            refresh_red_alert_ui,
            rotate_needle_by_direction,
            handle_direction_press,
            handle_red_alert_press,
        ));
    }
}

// ── Spawn system ──

/// Spawns the full captain panel once PhoneAssets are loaded.
fn spawn_captain_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    old_panel: Query<Entity, With<CaptainPanel>>,
) {
    let Some(assets) = assets else { return };

    // Despawn any stale captain panel from a previous run.
    for entity in old_panel.iter() {
        commands.entity(entity).despawn();
    }

    commands.insert_resource(CaptainPanelSpawned);

    let dial_radius = COMPASS_DIAMETER / 2.0;
    let btn_off = dial_radius - PAD_BTN_SIZE / 2.0 - 4.0;

    commands
        .spawn((
            CaptainPanel,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                top: Val::Px(44.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::FlexStart,
                padding: UiRect::top(Val::Px(28.0)),
                row_gap: Val::Px(COMPASS_TO_ALERT_GAP),
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|root| {
            // ── Compass dial ────────────────────────────────────────────
            root.spawn((CompassDial, Node {
                width: Val::Px(COMPASS_DIAMETER),
                height: Val::Px(COMPASS_DIAMETER),
                position_type: PositionType::Relative,
                ..default()
            })).with_children(|dial| {
                // Compass ring image (background)
                dial.spawn((
                    ImageNode::new(assets.compass_ring.clone()),
                    Node {
                        width: Val::Px(COMPASS_DIAMETER),
                        height: Val::Px(COMPASS_DIAMETER),
                        position_type: PositionType::Absolute,
                        left: Val::Px(0.0),
                        top: Val::Px(0.0),
                        ..default()
                    },
                ));

                // Cardinal letters
                let cardinals = [
                    ("F", 0.0, -dial_radius + 18.0),
                    ("P", -dial_radius + 18.0, 0.0),
                    ("S", dial_radius - 18.0, 0.0),
                    ("A", 0.0, dial_radius - 18.0),
                ];
                for &(letter, lx, ly) in &cardinals {
                    dial.spawn((
                        Text::new(letter),
                        TextFont {
                            font: assets.font_mono.clone(),
                            font_size: 16.0,
                            ..default()
                        },
                        TextColor(Color::srgba(0.55, 0.70, 1.0, 0.9)),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(dial_radius + lx - 5.0),
                            top: Val::Px(dial_radius + ly - 9.0),
                            ..default()
                        },
                    ));
                }

                // Rotating needle (centred in the dial)
                dial.spawn((
                    CompassNeedle,
                    ImageNode::new(assets.needle.clone()),
                    Node {
                        width: Val::Px(NEEDLE_SIZE),
                        height: Val::Px(NEEDLE_SIZE),
                        position_type: PositionType::Absolute,
                        left: Val::Px(dial_radius - NEEDLE_SIZE / 2.0),
                        top: Val::Px(dial_radius - NEEDLE_SIZE / 2.0),
                        ..default()
                    },
                    Transform::default(),
                ));

                // Direction pad buttons at the 4 cardinal points
                let buttons = [
                    (ViewDirection::Fore, "▲", "FWD", 0.0, -btn_off),
                    (ViewDirection::Port, "◄", "PORT", -btn_off, 0.0),
                    (ViewDirection::Starboard, "►", "STBD", btn_off, 0.0),
                    (ViewDirection::Aft, "▼", "AFT", 0.0, btn_off),
                ];
                for (dir, glyph, label, bx, by) in &buttons {
                    spawn_dir_button(dial, dir.clone(), glyph, label, dial_radius + bx, dial_radius + by);
                }
            });

            // ── Red Alert toggle ────────────────────────────────────────
            root.spawn((
                RedAlertToggle,
                Button,
                Node {
                    padding: UiRect::axes(Val::Px(20.0), Val::Px(12.0)),
                    column_gap: Val::Px(8.0),
                    align_items: AlignItems::Center,
                    ..default()
                },
                BackgroundColor(RA_BG_IDLE),
            )).with_children(|btn| {
                // Armed glow dot
                btn.spawn((
                    ArmedGlow,
                    Node {
                        width: Val::Px(10.0),
                        height: Val::Px(10.0),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(1.0, 0.2, 0.2, 0.0)),
                ));
                // Label
                btn.spawn((
                    Text::new("RED ALERT"),
                    TextFont {
                        font: assets.font_display.clone(),
                        font_size: 14.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.93, 0.93, 1.0)),
                ));
            });
        });
}

/// Spawn one chamfered direction pad button at the given absolute position.
fn spawn_dir_button(
    parent: &mut ChildSpawnerCommands,
    dir: ViewDirection,
    glyph: &str,
    label: &str,
    left: f32,
    top: f32,
) {
    parent.spawn((
        DirButton(dir),
        Button,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(left),
            top: Val::Px(top),
            width: Val::Px(PAD_BTN_SIZE),
            height: Val::Px(PAD_BTN_SIZE),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: Val::Px(2.0),
            border: UiRect::all(Val::Px(1.0)),
            ..default()
        },
        BackgroundColor(DIR_BG_IDLE),
        BorderColor::all(DIR_BORDER),
    )).with_children(|btn| {
        btn.spawn((
            Text::new(glyph),
            TextFont { font_size: 18.0, ..default() },
            TextColor(GLYPH_COLOR),
        ));
        btn.spawn((
            Text::new(label),
            TextFont { font_size: 8.0, ..default() },
            TextColor(LABEL_COLOR),
        ));
        btn.spawn((
            DirLed,
            Node {
                width: Val::Px(LED_SIZE),
                height: Val::Px(LED_SIZE),
                ..default()
            },
            BackgroundColor(LED_OFF),
        ));
    });
}

// ── Pure helpers ──

/// Returns the Z-axis rotation for the compass needle pointing in the given
/// direction.  Fore = 0° (up), Port = 90° CCW (left),
/// Starboard = −90° CCW (right), Aft = 180° (down).
pub fn needle_rotation(dir: &ViewDirection) -> Quat {
    match dir {
        ViewDirection::Fore => Quat::from_rotation_z(0.0),
        ViewDirection::Port => Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
        ViewDirection::Starboard => Quat::from_rotation_z(-std::f32::consts::FRAC_PI_2),
        ViewDirection::Aft => Quat::from_rotation_z(std::f32::consts::PI),
    }
}

/// Build a `SetView { Camera(direction) }` message for a direction pad press.
pub fn direction_press_message(dir: ViewDirection) -> ClientMessage {
    ClientMessage::SetView { mode: ViewMode::Camera(dir) }
}

// ── Systems ──

/// Highlights the active direction button's background and LED.
fn refresh_dir_highlights(
    sim: Res<ClientSimState>,
    mut buttons: Query<(&DirButton, &Children, &mut BackgroundColor), Without<DirLed>>,
    mut leds: Query<&mut BackgroundColor, With<DirLed>>,
) {
    if !sim.is_changed() {
        return;
    }
    for (dir_btn, children, mut bg) in buttons.iter_mut() {
        let active = sim.is_active_camera_direction(&dir_btn.0);
        bg.0 = if active { DIR_BG_ACTIVE } else { DIR_BG_IDLE };

        for child in children.iter() {
                if let Ok(mut led_bg) = leds.get_mut(child) {
                led_bg.0 = if active { LED_ON } else { LED_OFF };
            }
        }
    }
}

/// Updates the red alert toggle background and the armed glow pulse.
fn refresh_red_alert_ui(
    sim: Option<Res<ClientSimState>>,
    time: Res<Time>,
    mut toggle_q: Query<&mut BackgroundColor, (With<RedAlertToggle>, Without<ArmedGlow>)>,
    mut glow_q: Query<&mut BackgroundColor, With<ArmedGlow>>,
) {
    let Some(sim) = sim else { return };
    if !sim.is_changed() && !sim.red_alert {
        return;
    }

    for mut bg in toggle_q.iter_mut() {
        bg.0 = if sim.red_alert { RA_BG_ACTIVE } else { RA_BG_IDLE };
    }

    for mut glow in glow_q.iter_mut() {
        if sim.red_alert {
            let pulse = ((time.elapsed_secs() * 3.0).sin() + 1.0) * 0.3 + 0.2;
            glow.0 = Color::srgba(1.0, 0.2, 0.2, pulse);
        } else {
            glow.0 = Color::srgba(1.0, 0.2, 0.2, 0.0);
        }
    }
}

/// Rotates the compass needle to match the active `ViewDirection`.
fn rotate_needle_by_direction(
    sim: Res<ClientSimState>,
    mut needles: Query<&mut Transform, With<CompassNeedle>>,
) {
    let dir = match &sim.view_mode {
        ViewMode::Camera(d) => d,
        _ => return,
    };
    let rotation = needle_rotation(dir);
    for mut tf in needles.iter_mut() {
        tf.rotation = rotation;
    }
}

/// Handles direction pad button presses and emits `SetView`.
fn handle_direction_press(
    mut interactions: Query<(&Interaction, &DirButton), (Changed<Interaction>, With<Button>)>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, dir_btn) in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(direction_press_message(dir_btn.0.clone())));
        }
    }
}

/// Handles red alert toggle presses and emits `ToggleRedAlert`.
fn handle_red_alert_press(
    mut interactions: Query<
        &Interaction,
        (Changed<Interaction>, With<Button>, With<RedAlertToggle>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(ClientMessage::ToggleRedAlert));
        }
    }
}

// ── Tests ──

#[cfg(test)]
mod tests {
    use super::*;

    // ── needle_rotation ─────────────────────────────────────────────

    #[test]
    fn needle_fore_no_rotation() {
        let rot = needle_rotation(&ViewDirection::Fore);
        let (_axis, angle) = rot.to_axis_angle();
        assert!(
            (angle - 0.0).abs() < 1e-6,
            "fore should be 0° rotation, got {angle}"
        );
    }

    #[test]
    fn needle_port_rotates_pos_90() {
        let rot = needle_rotation(&ViewDirection::Port);
        let (axis, angle) = rot.to_axis_angle();
        assert!(
            (angle - std::f32::consts::FRAC_PI_2).abs() < 1e-6,
            "port should be +90°, got {angle}"
        );
        assert!(axis.z > 0.0, "port axis should have positive z");
    }

    #[test]
    fn needle_starboard_rotates_neg_90() {
        let rot = needle_rotation(&ViewDirection::Starboard);
        let (axis, angle) = rot.to_axis_angle();
        // `to_axis_angle` normalises negative angles to [0, π] with
        // reversed axis, so we check the angle magnitude and z sign.
        assert!(
            (angle - std::f32::consts::FRAC_PI_2).abs() < 1e-6,
            "starboard should be 90° magnitude, got {angle}"
        );
        assert!(axis.z < 0.0, "starboard axis should have negative z");
    }

    #[test]
    fn needle_aft_rotates_180() {
        let rot = needle_rotation(&ViewDirection::Aft);
        let (_axis, angle) = rot.to_axis_angle();
        assert!(
            (angle - std::f32::consts::PI).abs() < 1e-6,
            "aft should be 180°, got {angle}"
        );
    }

    // ── direction_press_message ─────────────────────────────────────

    #[test]
    fn direction_press_builds_set_view() {
        let msg = direction_press_message(ViewDirection::Port);
        assert_eq!(
            msg,
            ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Port) }
        );
    }

    #[test]
    fn direction_press_message_for_fore() {
        let msg = direction_press_message(ViewDirection::Fore);
        assert_eq!(
            msg,
            ClientMessage::SetView { mode: ViewMode::Camera(ViewDirection::Fore) }
        );
    }
}
