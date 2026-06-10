//! Client-side Captain Panel plugin — migrated to `src/gui/` library widgets.
//!
//! Owns all captain console UI: compass dial with rotating needle, direction-pad
//! buttons (each a `GuiButton` with `RadioMember`), and Red Alert `GuiButton`.
//!
//! No per-button marker-component query systems remain. All callbacks are wired
//! via observers at spawn time:
//!
//! - Direction buttons: `on_radio_member_pressed` (from `gui::radio`) fires
//!   `RadioSelected` on the group; `on_dir_selected` sends `SetView`.
//! - Red Alert button: `on_red_alert_pressed` writes `RedAlertIntensity` and
//!   sends `ToggleRedAlert`.

use bevy::prelude::*;

use crate::client::console_shell::ConsoleShell;
use crate::client_app::{CaptainPanel, OutboundClientMessage};
use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::gui::{
    on_radio_member_pressed, spawn_gui_button, ButtonPressed, ButtonSize, RadioGroupMarker,
    RadioMember, RadioSelected, RedAlertIntensity, StateVisuals, Visual, WidgetState,
};
use crate::messages::{ClientMessage, Console, GamePhase, ViewDirection, ViewMode};
use crate::phone_border::framing::{DeviceOrientation, PhoneAssets};
use crate::ship_view::ShipView;

// ── Constants ──

/// Diameter of the compass dial in logical pixels.
const COMPASS_DIAMETER: f32 = 240.0;

/// Size of each direction pad button.
const PAD_BTN_SIZE: f32 = 52.0;

/// Size of the LED indicator dot.
const LED_SIZE: f32 = 8.0;

/// Size of the needle image.
const NEEDLE_SIZE: f32 = 64.0;

// ── Colours ──

const LED_OFF: Color = Color::srgb(0.12, 0.12, 0.22);
const LED_ON: Color = Color::srgb(0.2, 0.9, 0.2);
const GLYPH_COLOR: Color = Color::srgb(0.90, 0.90, 1.0);
const LABEL_COLOR: Color = Color::srgb(0.55, 0.65, 0.70);
const RA_BG_IDLE: Color = Color::srgb(0.13, 0.13, 0.27);
const RA_BG_ACTIVE: Color = Color::srgb(0.40, 0.0, 0.0);

// ── Marker components ──

/// Marks the compass dial root.
#[derive(Component)]
pub struct CompassDial;

/// Marks the rotating compass needle.
#[derive(Component)]
pub struct CompassNeedle;

/// Marks an LED indicator dot inside a direction pad button.
#[derive(Component)]
pub struct DirLed;

/// Marks the armed glow indicator on the red alert button.
#[derive(Component)]
pub struct ArmedGlow;

/// Lightweight component on each direction-pad `GuiButton` that stores which
/// `ViewDirection` it represents.  Read only in the `RadioSelected` observer —
/// no query-driven system polls it.
#[derive(Component, Clone, Debug, PartialEq)]
pub struct DirChoice(pub ViewDirection);

/// Marks the RadioGroup entity used for the direction pad.
#[derive(Component)]
struct DirRadioGroup;

/// One-shot resource set after the captain UI is first spawned.
#[derive(Resource)]
pub struct CaptainPanelSpawned;

// ── Plugin ──

pub struct CaptainPanelPlugin;

impl Plugin for CaptainPanelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (
            spawn_captain_ui.run_if(not(resource_exists::<CaptainPanelSpawned>)),
            toggle_captain_panel_visibility,
            refresh_red_alert_ui,
            sync_dir_radio_active_state,
            rotate_needle_by_direction,
            respawn_captain_on_orientation_change,
        ));
    }
}

// ── Pure helpers ──

/// Returns whether the captain panel should be visible, given the current
/// lobby state, local player token, and active console override.
///
/// Visibility rules:
/// - Game must be in progress.
/// - Local player must hold `CaptainChair`.
/// - If the player holds only one console, always show it.
/// - If the player holds multiple consoles, only show when the active tab
///   is explicitly set to `CaptainChair`.
pub fn captain_panel_visible(
    lobby: &LobbyState,
    token: &str,
    active: &ActiveConsole,
) -> bool {
    if lobby.phase != GamePhase::InProgress {
        return false;
    }
    let view = LobbyView::new(lobby, token);
    if !view.is_captain() {
        return false;
    }
    let my_consoles_count = view.my_consoles().len();
    match &active.0 {
        Some(c) => *c == Console::CaptainChair,
        None => my_consoles_count == 1,
    }
}

// ── Systems ──

fn toggle_captain_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<CaptainPanel>>,
) {
    let visible = captain_panel_visible(&lobby, &token.0, &active);
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

// ── Spawn system ──

fn spawn_captain_ui(
    mut commands: Commands,
    assets: Option<Res<PhoneAssets>>,
    old_panel: Query<Entity, With<CaptainPanel>>,
    old_help: Query<(Entity, &crate::client::elements::HelpOverlay)>,
    orientation: Option<Res<DeviceOrientation>>,
) {
    let Some(assets) = assets else { return };
    let is_landscape = crate::phone_border::framing::is_landscape(orientation.as_deref());

    for entity in old_panel.iter() {
        commands.entity(entity).despawn();
    }
    // Despawn any stale Captain help-overlay from a previous spawn (e.g. an
    // orientation respawn) before ConsoleShell::spawn creates a fresh one.
    for (entity, overlay) in old_help.iter() {
        if overlay.0 == crate::client::elements::HelpPanel::CaptainChair {
            commands.entity(entity).despawn();
        }
    }

    commands.insert_resource(CaptainPanelSpawned);

    let shell = ConsoleShell::spawn(
        &mut commands,
        assets.captain_panel_bg.clone(),
        is_landscape,
        crate::client::elements::HelpPanel::CaptainChair,
        |commands: &mut Commands, primary: Entity| {
            fill_captain_dirpad(commands, primary, &assets);
        },
        |commands: &mut Commands, secondary: Entity| {
            fill_captain_alert(commands, secondary, &assets);
        },
        &assets,
    );

    commands.entity(shell.root).insert(CaptainPanel);
}

// ── Fill helpers ──

/// Builds the compass dial + direction-pad RadioGroup inside `container`.
///
/// The four direction buttons are `GuiButton`s with `RadioMember` attached,
/// positioned absolutely within the compass dial.  `on_radio_member_pressed`
/// (from `gui::radio`) fires `RadioSelected` on the group; `on_dir_selected`
/// translates that into a `SetView` outbound message.
fn fill_captain_dirpad(commands: &mut Commands, container: Entity, assets: &PhoneAssets) {
    let dial_radius = COMPASS_DIAMETER / 2.0;
    let btn_off = dial_radius - PAD_BTN_SIZE / 2.0 - 4.0;

    // Column wrapper to center the compass dial in the primary slot
    let col = commands.spawn(Node {
        flex_direction: FlexDirection::Column,
        align_items: AlignItems::Center,
        justify_content: JustifyContent::Center,
        width: Val::Percent(100.0),
        height: Val::Percent(100.0),
        ..default()
    }).id();
    commands.entity(container).add_child(col);

    let dial = commands.spawn((CompassDial, Node {
        width: Val::Px(COMPASS_DIAMETER),
        height: Val::Px(COMPASS_DIAMETER),
        position_type: PositionType::Relative,
        ..default()
    })).id();
    commands.entity(col).add_child(dial);

    // Compass ring image (background)
    let ring = commands.spawn((
        ImageNode::new(assets.compass_ring.clone()),
        Node {
            width: Val::Px(COMPASS_DIAMETER),
            height: Val::Px(COMPASS_DIAMETER),
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            ..default()
        },
    )).id();
    commands.entity(dial).add_child(ring);

    // Cardinal letters
    let cardinals = [
        ("F", 0.0, -dial_radius + 18.0),
        ("P", -dial_radius + 18.0, 0.0),
        ("S", dial_radius - 18.0, 0.0),
        ("A", 0.0, dial_radius - 18.0),
    ];
    for &(letter, lx, ly) in &cardinals {
        let label = commands.spawn((
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
        )).id();
        commands.entity(dial).add_child(label);
    }

    // Rotating needle
    let needle_wrapper = commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(dial_radius - NEEDLE_SIZE / 2.0),
            top: Val::Px(dial_radius - NEEDLE_SIZE / 2.0),
            width: Val::Px(NEEDLE_SIZE),
            height: Val::Px(NEEDLE_SIZE),
            ..default()
        },
        Transform::default(),
        GlobalTransform::default(),
    )).id();
    commands.entity(dial).add_child(needle_wrapper);

    let needle = commands.spawn((
        CompassNeedle,
        ImageNode::new(assets.needle.clone()),
        Transform::default(),
        GlobalTransform::default(),
    )).id();
    commands.entity(needle_wrapper).add_child(needle);

    // ── Direction RadioGroup (zero-size group container) ──────────────
    //
    // The group itself is zero-size because the member buttons are positioned
    // absolutely within the compass dial rather than laid out in a row.
    // Each button gets a `RadioMember { group }` component and an
    // `on_radio_member_pressed` observer.  `RadioGroupMarker` is required so
    // `RadioGroup`-compatible queries in radio.rs find the group.
    let dir_group = commands.spawn((
        DirRadioGroup,
        RadioGroupMarker,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            top: Val::Px(0.0),
            width: Val::Px(0.0),
            height: Val::Px(0.0),
            ..default()
        },
    ))
    .observe(|_: On<RadioSelected>| {})   // stub; real handler added below
    .id();

    // Attach the direction-selection observer on the group.
    commands.entity(dir_group).observe(on_dir_selected);

    commands.entity(dial).add_child(dir_group);

    // Direction pad buttons at the 4 cardinal points
    let button_defs: [(ViewDirection, &str, &str, f32, f32); 4] = [
        (ViewDirection::Fore,      "▲", "FWD",  0.0,     -btn_off),
        (ViewDirection::Port,      "◄", "PORT", -btn_off, 0.0),
        (ViewDirection::Starboard, "►", "STBD",  btn_off, 0.0),
        (ViewDirection::Aft,       "▼", "AFT",   0.0,     btn_off),
    ];

    for (dir, glyph, lbl, bx, by) in button_defs {
        let dir_state_visuals = StateVisuals {
            idle: Visual {
                image: Some(assets.btn_small_idle.clone()),
                color: Color::srgb(0.12, 0.12, 0.26),
            },
            hover: Visual {
                image: Some(assets.btn_small_hover.clone()),
                color: Color::srgb(0.16, 0.18, 0.35),
            },
            active: Visual {
                image: Some(assets.btn_small_active.clone()),
                color: Color::srgb(0.20, 0.28, 0.50),
            },
            press: Visual {
                image: Some(assets.btn_small_press.clone()),
                color: Color::srgb(0.25, 0.35, 0.60),
            },
            disabled: Visual {
                image: None,
                color: Color::srgba(0.08, 0.08, 0.15, 0.5),
            },
        };

        let btn = spawn_gui_button(
            commands,
            ButtonSize::Square(PAD_BTN_SIZE),
            dir_state_visuals,
        );

        // Pre-select Fore (matches ViewMode default).
        let initial_active = dir == ViewDirection::Fore;
        commands.entity(btn).insert((
            DirChoice(dir),
            RadioMember { group: dir_group },
            WidgetState { active: initial_active },
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(dial_radius + bx - PAD_BTN_SIZE / 2.0),
                top:  Val::Px(dial_radius + by - PAD_BTN_SIZE / 2.0),
                width: Val::Px(PAD_BTN_SIZE),
                height: Val::Px(PAD_BTN_SIZE),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(2.0),
                ..default()
            },
        ));

        // Wire the RadioGroup member observer so pressing the button fires
        // RadioSelected on the group entity.
        commands.entity(btn).observe(on_radio_member_pressed);

        // Glyph, label, LED children
        let glyph_node = commands.spawn((
            Text::new(glyph),
            TextFont { font_size: 18.0, ..default() },
            TextColor(GLYPH_COLOR),
        )).id();
        let label_node = commands.spawn((
            Text::new(lbl),
            TextFont { font_size: 8.0, ..default() },
            TextColor(LABEL_COLOR),
        )).id();
        let led_node = commands.spawn((
            DirLed,
            Node {
                width: Val::Px(LED_SIZE),
                height: Val::Px(LED_SIZE),
                ..default()
            },
            BackgroundColor(if initial_active { LED_ON } else { LED_OFF }),
        )).id();
        commands.entity(btn).add_child(glyph_node);
        commands.entity(btn).add_child(label_node);
        commands.entity(btn).add_child(led_node);

        commands.entity(dial).add_child(btn);
    }
}

/// Builds the Red Alert `GuiButton` inside `container`.
///
/// On press the `on_red_alert_pressed` observer fires `ToggleRedAlert` and
/// writes a provisional `RedAlertIntensity` for immediate visual feedback.
/// `refresh_red_alert_ui` keeps the button's `WidgetState.active` and the
/// `ArmedGlow` child in sync with the server-confirmed `ShipView.red_alert`.
fn fill_captain_alert(commands: &mut Commands, container: Entity, assets: &PhoneAssets) {
    // Centering wrapper — keeps the button at its intrinsic size instead
    // of stretching to fill the secondary slot.
    let center = commands.spawn((
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            padding: UiRect::all(Val::Px(16.0)),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
    )).id();
    commands.entity(container).add_child(center);

    let ra_visuals = StateVisuals {
        idle: Visual {
            image: Some(assets.red_alert_idle.clone()),
            color: RA_BG_IDLE,
        },
        hover: Visual {
            image: Some(assets.red_alert_hover.clone()),
            color: RA_BG_IDLE,
        },
        active: Visual {
            image: Some(assets.red_alert_active.clone()),
            color: RA_BG_ACTIVE,
        },
        press: Visual {
            image: Some(assets.red_alert_press.clone()),
            color: RA_BG_IDLE,
        },
        disabled: Visual {
            image: None,
            color: Color::srgba(0.08, 0.08, 0.15, 0.5),
        },
    };

    let ra_btn = spawn_gui_button(
        commands,
        ButtonSize::Rect { width: 160.0, height: 52.0 },
        ra_visuals,
    );

    commands.entity(ra_btn).insert(Node {
        padding: UiRect::axes(Val::Px(20.0), Val::Px(12.0)),
        column_gap: Val::Px(8.0),
        align_items: AlignItems::Center,
        flex_grow: 0.0,
        flex_shrink: 0.0,
        align_self: AlignSelf::Center,
        ..default()
    });

    let glow = commands.spawn((
        ArmedGlow,
        ImageNode {
            image: assets.red_alert_armed.clone(),
            color: Color::srgba(1.0, 1.0, 1.0, 0.0).into(),
            ..default()
        },
        Node {
            width: Val::Px(16.0),
            height: Val::Px(16.0),
            ..default()
        },
    )).id();
    let text = commands.spawn((
        Text::new("RED ALERT"),
        TextFont {
            font: assets.font_display.clone(),
            font_size: 14.0,
            ..default()
        },
        TextColor(Color::srgb(0.93, 0.93, 1.0)),
    )).id();
    commands.entity(ra_btn).add_child(glow);
    commands.entity(ra_btn).add_child(text);

    commands.entity(ra_btn).observe(on_red_alert_pressed);
    commands.entity(center).add_child(ra_btn);
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

// ── Observer: direction RadioGroup selected ──

/// Fired on the `DirRadioGroup` entity when a member button is pressed.
/// Looks up the `DirChoice` on the selected member entity, sends `SetView`,
/// and optimistically updates `ShipView` so the button lights up immediately.
/// `pending_view_mode` protects the optimistic state from being overwritten
/// by stale `SimState` broadcasts until the server confirms the change.
fn on_dir_selected(
    trigger: On<RadioSelected>,
    dir_choices: Query<&DirChoice>,
    mut outbound: MessageWriter<OutboundClientMessage>,
    mut ship_view: ResMut<ShipView>,
) {
    let member = trigger.event().member;
    let Ok(choice) = dir_choices.get(member) else { return };
    let dir = choice.0.clone();
    outbound.write(OutboundClientMessage(direction_press_message(dir.clone())));
    let new_mode = ViewMode::Camera(dir);
    ship_view.pending_view_mode = Some(new_mode.clone());
    ship_view.view_mode = new_mode;
}

// ── Observer: Red Alert button pressed ──

/// Fired when the Red Alert `GuiButton` is pressed.
///
/// Sends `ToggleRedAlert` to the server and applies an immediate provisional
/// `RedAlertIntensity` so the border/vignette responds before the server
/// echoes the confirmed state back.
fn on_red_alert_pressed(
    _trigger: On<ButtonPressed>,
    mut outbound: MessageWriter<OutboundClientMessage>,
    mut intensity: ResMut<RedAlertIntensity>,
    ship_view: Option<Res<ShipView>>,
) {
    outbound.write(OutboundClientMessage(ClientMessage::ToggleRedAlert));
    // Optimistic intensity update so the bezel reacts immediately.
    if let Some(sv) = ship_view {
        if sv.red_alert {
            // Turning off — clear immediately.
            intensity.0 = 0.0;
        } else {
            // Turning on — seed a modest value; pulse system takes over next frame.
            intensity.0 = 0.5;
        }
    }
}

// ── System: sync WidgetState + ArmedGlow from ShipView ──

/// Drives the Red Alert button's `WidgetState.active` (→ button color via
/// `resolve_visuals_system`) and the pulsing `ArmedGlow` indicator from the
/// server-confirmed `ShipView.red_alert`.
///
/// The armed-glow pulse preserves the pre-migration visual effect.
fn refresh_red_alert_ui(
    ship_view: Option<Res<ShipView>>,
    time: Res<Time>,
    mut ra_states: Query<&mut WidgetState, With<crate::gui::GuiButtonMarker>>,
    mut glow_q: Query<(&mut ImageNode, &ChildOf), With<ArmedGlow>>,
) {
    let Some(ship_view) = ship_view else { return };
    if !ship_view.is_changed() && !ship_view.red_alert {
        return;
    }

    // Update WidgetState only for the button that has ArmedGlow as a child.
    // We identify the RA button by querying ArmedGlow and walking up to the parent.
    for (mut glow_img, parent) in glow_q.iter_mut() {
        let ra_entity = parent.0;
        if let Ok(mut state) = ra_states.get_mut(ra_entity) {
            state.active = ship_view.red_alert;
        }

        // Pulsing armed glow — fade the texture alpha.
        if ship_view.red_alert {
            let pulse = ((time.elapsed_secs() * 3.0).sin() + 1.0) * 0.3 + 0.2;
            glow_img.color = Color::srgba(1.0, 1.0, 1.0, pulse).into();
        } else {
            glow_img.color = Color::srgba(1.0, 1.0, 1.0, 0.0).into();
        }
    }
}

// ── System: sync direction pad active states + LEDs from ShipView ──

/// Drives `WidgetState.active` and the `DirLed` child color on direction-pad
/// buttons from `ShipView.view_mode`.  Runs whenever `ShipView` changes.
fn sync_dir_radio_active_state(
    ship_view: Res<ShipView>,
    mut buttons: Query<(&DirChoice, &mut WidgetState, &Children)>,
    mut leds: Query<&mut BackgroundColor, With<DirLed>>,
) {
    let active_dir = match &ship_view.view_mode {
        ViewMode::Camera(d) => Some(d.clone()),
        _ => None,
    };
    for (choice, mut state, children) in buttons.iter_mut() {
        let is_active = active_dir.as_ref().map(|d| *d == choice.0).unwrap_or(false);
        state.active = is_active;

        // Update the LED child.
        for child in children.iter() {
            if let Ok(mut led_bg) = leds.get_mut(child) {
                led_bg.0 = if is_active { LED_ON } else { LED_OFF };
            }
        }
    }
}

// ── System: rotate compass needle ──

fn rotate_needle_by_direction(
    ship_view: Res<ShipView>,
    mut needles: Query<&mut Transform, With<CompassNeedle>>,
) {
    let dir = match &ship_view.view_mode {
        ViewMode::Camera(d) => d,
        _ => return,
    };
    let rotation = needle_rotation(dir);
    for mut tf in needles.iter_mut() {
        tf.rotation = rotation;
    }
}

// ── Orientation respawn ──

/// When `DeviceOrientation` changes, despawn the current `CaptainPanel` and
/// remove the `CaptainPanelSpawned` resource so `spawn_captain_ui` respawns
/// with the correct layout.
fn respawn_captain_on_orientation_change(
    orientation: Res<DeviceOrientation>,
    panel: Query<Entity, With<CaptainPanel>>,
    mut commands: Commands,
) {
    if !orientation.is_changed() || orientation.is_added() {
        return;
    }
    for entity in panel.iter() {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<CaptainPanelSpawned>();
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

    // ── captain_panel_visible ───────────────────────────────────────

    use crate::messages::{GameState, Player, ServerMessage, ShipClientConfig};
    use crate::stations_config::ShipStations;
    use std::collections::HashMap;

    fn player(token: &str, consoles: Vec<Console>) -> Player {
        Player { token: token.into(), name: "test".into(), consoles, connected: true }
    }

    fn game_state(phase: GamePhase, players: Vec<Player>) -> GameState {
        GameState { phase, players, complexity: HashMap::new(), world: None }
    }

    fn welcome(state: GameState) -> ServerMessage {
        ServerMessage::Welcome {
            state,
            ship_stations: ShipStations::default(),
            ship_config: ShipClientConfig::default(),
        }
    }

    fn in_progress_lobby(captain_token: &str) -> LobbyState {
        let mut s = LobbyState::default();
        s.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player(captain_token, vec![Console::CaptainChair])],
        )));
        s
    }

    #[test]
    fn hidden_during_lobby_phase() {
        let lobby = LobbyState::default();
        let active = ActiveConsole::default();
        assert!(!captain_panel_visible(&lobby, "me", &active));
    }

    #[test]
    fn hidden_when_player_does_not_hold_captain_chair() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("me", vec![Console::Helm])],
        )));
        let active = ActiveConsole::default();
        assert!(!captain_panel_visible(&lobby, "me", &active));
    }

    #[test]
    fn visible_when_captain_and_only_console() {
        let lobby = in_progress_lobby("me");
        let active = ActiveConsole::default();
        assert!(captain_panel_visible(&lobby, "me", &active));
    }

    #[test]
    fn hidden_when_captain_but_tab_set_to_other_console() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("me", vec![Console::CaptainChair, Console::Helm])],
        )));
        let active = ActiveConsole(Some(Console::Helm));
        assert!(!captain_panel_visible(&lobby, "me", &active));
    }

    #[test]
    fn visible_when_captain_and_tab_explicitly_set_to_captain_chair() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("me", vec![Console::CaptainChair, Console::Helm])],
        )));
        let active = ActiveConsole(Some(Console::CaptainChair));
        assert!(captain_panel_visible(&lobby, "me", &active));
    }

    #[test]
    fn hidden_when_multi_console_captain_and_no_tab_set() {
        let mut lobby = LobbyState::default();
        lobby.apply(&welcome(game_state(
            GamePhase::InProgress,
            vec![player("me", vec![Console::CaptainChair, Console::Helm])],
        )));
        let active = ActiveConsole::default();
        assert!(!captain_panel_visible(&lobby, "me", &active));
    }

    // ── DirChoice ───────────────────────────────────────────────────

    #[test]
    fn dir_choice_stores_direction() {
        let choice = DirChoice(ViewDirection::Fore);
        assert_eq!(choice.0, ViewDirection::Fore);
    }

    #[test]
    fn dir_choice_all_directions_are_distinct() {
        let fore      = DirChoice(ViewDirection::Fore);
        let aft       = DirChoice(ViewDirection::Aft);
        let port      = DirChoice(ViewDirection::Port);
        let starboard = DirChoice(ViewDirection::Starboard);
        assert_ne!(fore.0, aft.0);
        assert_ne!(fore.0, port.0);
        assert_ne!(fore.0, starboard.0);
        assert_ne!(aft.0, port.0);
    }

    // ── StateVisuals: five states render distinctly ─────────────────

    #[test]
    fn dir_visuals_active_differs_from_idle() {
        use crate::gui::resolve_visual;
        let v = StateVisuals::from_colors(
            Color::srgb(0.12, 0.12, 0.26),       // idle
            Color::srgb(0.16, 0.18, 0.35),       // hover
            Color::srgb(0.20, 0.28, 0.50),       // active
            Color::srgb(0.25, 0.35, 0.60),       // press
            Color::srgba(0.08, 0.08, 0.15, 0.5), // disabled
        );
        let idle   = resolve_visual(&v, false, false, false, false).color;
        let active = resolve_visual(&v, false, false, true,  false).color;
        assert_ne!(idle, active, "active direction button should differ from idle");
    }

    #[test]
    fn ra_visuals_active_differs_from_idle() {
        use crate::gui::resolve_visual;
        let v = StateVisuals::from_colors(
            RA_BG_IDLE,
            RA_BG_IDLE,
            RA_BG_ACTIVE,
            RA_BG_IDLE,
            Color::srgba(0.08, 0.08, 0.15, 0.5),
        );
        let idle   = resolve_visual(&v, false, false, false, false).color;
        let active = resolve_visual(&v, false, false, true,  false).color;
        assert_ne!(idle, active, "active RA button should differ from idle");
    }

    #[test]
    fn ra_visuals_five_states_all_present() {
        use crate::gui::resolve_visual;
        let v = StateVisuals {
            idle:     Visual { image: None, color: RA_BG_IDLE },
            hover:    Visual { image: None, color: RA_BG_IDLE },
            active:   Visual { image: None, color: RA_BG_ACTIVE },
            press:    Visual { image: None, color: RA_BG_IDLE },
            disabled: Visual { image: None, color: Color::srgba(0.08, 0.08, 0.15, 0.5) },
        };
        let idle     = resolve_visual(&v, false, false, false, false).color;
        let active   = resolve_visual(&v, false, false, true,  false).color;
        let disabled = resolve_visual(&v, true,  false, false, false).color;
        assert_ne!(idle, active);
        assert_ne!(idle, disabled);
        assert_ne!(active, disabled);
    }
}
