//! Client-side Bevy app — lobby UI and (future) in-game UI.
//!
//! This plugin owns the `LobbyState` and `LocalPlayerToken` resources,
//! drains inbound `ServerMessage` events, re-renders the lobby UI on
//! state changes, and emits outbound `ClientMessage` events when buttons
//! are pressed. Outbound emission is the only side effect that escapes
//! the plugin; the bridge layer (`client_bridge`) is responsible for
//! marshalling those events to/from JS.
//!
//! The plugin is platform-agnostic; the bridge layer wires it together
//! with `DefaultPlugins` and the wasm-bindgen entry points.

use bevy::prelude::*;

use crate::client_helm::{release, tick, HelmJoystickState};
use crate::client_lobby::{
    engage_message, message_for_station_slot_click, reconcile_active_console, StationSlot,
    LobbyState, LobbyView, LocalPlayerToken, ActiveConsole,
};
use crate::client_sim::{
    message_for_direction_press, on_screen_message, red_alert_toggle_message,
    fire_phaser_message, set_phaser_mode_message, fire_torpedo_message, ClientSimState,
};
use crate::messages::{ClientMessage, Console, GamePhase, PhaserMode, ServerMessage, Shape, ViewDirection};

// ── Events ─────────────────────────────────────────────────────────

/// Fired by the bridge each time JS hands the WASM client an inbound
/// `ServerMessage`. The plugin consumes these to update `LobbyState`.
#[derive(Message, Clone, Debug)]
pub struct InboundServerMessage(pub ServerMessage);

/// Fired by the plugin whenever a UI interaction needs to send a
/// `ClientMessage` to the host. The bridge layer drains these and
/// forwards JSON over the JS callback.
#[derive(Message, Clone, Debug)]
pub struct OutboundClientMessage(pub ClientMessage);

// ── Marker components ──────────────────────────────────────────────

/// Marks the root node of the lobby UI so it can be shown/hidden when
/// the phase changes, and reparented into the bezel content area.
#[derive(Component)]
pub struct LobbyRoot;

/// Marks the container of the per-console buttons so it can be cleared
/// and rebuilt on every `LobbyState` change.
#[derive(Component)]
struct ConsoleListRoot;

/// Marks the container of the player list lines.
#[derive(Component)]
struct PlayerListRoot;

/// Marks the Engage button so we can toggle its visibility per captaincy.
#[derive(Component)]
struct EngageButton;

/// Marks one station-row button and remembers which station name it acts on.
#[derive(Component)]
struct StationButton(String);

/// Marks the root of the captain console UI (view selector + Red Alert);
/// shown only when the local player holds CaptainChair and the phase is
/// InProgress.
#[derive(Component)]
pub struct CaptainPanel;

/// Marks one direction button in the view-selector cross.
#[derive(Component)]
struct ViewDirButton(ViewDirection);

/// Marks the Red Alert toggle button so its background and label can
/// reflect the current `ClientSimState.red_alert`.
#[derive(Component)]
struct RedAlertButton;

/// Marks the text node *inside* the Red Alert button so we can update
/// the "ON"/"OFF" label without rebuilding the button entity.
#[derive(Component)]
struct RedAlertLabel;

/// Marks the root of the helm joystick UI; shown only when the local
/// player holds Helm and the phase is InProgress.
#[derive(Component)]
pub struct HelmPanel;

/// Marks the radar panel container. Retained for the (now-inert)
/// `draw_helm_radar` gizmo system.
#[derive(Component)]
pub struct RadarPanel;

/// Marks the small movable knob nested inside the pad.
#[derive(Component)]
pub struct HelmKnob;

/// Marks the text node showing live "Thrust X% / Steering Y%" values.
#[derive(Component)]
pub struct HelmReadout;

/// Marks the "On Screen" button on the helm console; pressing it sends
/// `SetView { mode: Radar }` so the server viewscreen mirrors the radar.
#[derive(Component)]
pub struct OnScreenButton;

/// Marks the Repair button on the helm console.
#[derive(Component)]
pub struct RepairButton;

/// Marks the text label inside the Repair button (used to refresh cooldown text).
#[derive(Component)]
pub struct RepairButtonLabel;

/// Marks a text node that displays the current repair icon shape (or clearance).
/// Spawned on any panel that should show it (Helm, Tactical, Science at minimum).
#[derive(Component)]
pub struct RepairIconLabel;

/// Marks the root of the repair console UI; shown only when the local
/// player holds `Console::Repair` and the phase is InProgress.
#[derive(Component)]
pub struct RepairPanel;

/// Marks the text label that shows the current breakdown or "All Systems Nominal".
#[derive(Component)]
struct RepairBreakdownLabel;

/// Marks a shape button on the Repair console. Carries the shape it fires.
#[derive(Component)]
struct RepairShapeButton(Shape);

/// Marks the container for the three shape buttons, so its children can be
/// disabled/enabled together.
#[derive(Component)]
struct RepairShapeButtonRoot;

/// Marks a team row container (index 0, 1, or 2).
#[derive(Component)]
#[allow(dead_code)]
struct RepairTeamRow(usize);

/// Marks the progress-bar fill inside a team row.
#[derive(Component)]
struct RepairTeamFill(usize);

/// Marks the status text overlaid on a team row.
#[derive(Component)]
struct RepairTeamStatusText(usize);

/// Marks the root of the science console UI; shown only when the local
/// player holds Science and the phase is InProgress.
#[derive(Component)]
pub struct SciencePanel;

/// Marks the root of the weapons console UI; shown only when the local
/// player holds Tactical and the phase is InProgress.
#[derive(Component)]
pub struct WeaponsPanel;

/// Marks the root of the power console UI; shown only when the local
/// player holds Power and the phase is InProgress.
#[derive(Component)]
pub struct PowerPanel;

/// Marks one power allocation row container, carrying the console it controls.
#[derive(Component)]
struct PowerRow(Console);

/// Marks the label showing current level for a row (inside that row).
/// Carries the console it represents for refresh matching.
#[derive(Component)]
struct PowerRowLevel(Console);

/// Marks the increment button for a power row. Carries the target console.
#[derive(Component)]
struct PowerIncButton(Console);

/// Marks the decrement button for a power row. Carries the target console.
#[derive(Component)]
struct PowerDecButton(Console);

/// Marks the battery bar fill node.
#[derive(Component)]
struct BatteryBar;

/// Marks the battery percentage text label.
#[derive(Component)]
struct BatteryLabel;

/// Marks the "FIRE PHASERS" button on the Weapons console.
#[derive(Component)]
struct FirePhaserButton;

/// Marks the text label inside the Fire button (used to show cooldown status).
#[derive(Component)]
struct FirePhaserLabel;

/// Marks the phaser mode toggle button (Auto / Manual).
#[derive(Component)]
struct PhaserModeButton;

/// Marks the text label inside the mode button.
#[derive(Component)]
struct PhaserModeLabel;

/// Tracks which torpedo tube is currently selected on the Weapons console.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
struct SelectedTube(Option<crate::messages::TorpedoTube>);

/// Marks a torpedo tube selection button. Contains the tube it represents.
#[derive(Component)]
struct TorpedoTubeButton(crate::messages::TorpedoTube);

/// Marks the "FIRE TORPEDO" button on the Weapons console.
#[derive(Component)]
struct FireTorpedoButton;

/// Marks the text label inside the Fire Torpedo button.
#[derive(Component)]
struct FireTorpedoLabel;

/// Marks the torpedo count text label.
#[derive(Component)]
struct TorpedoCountLabel;

/// Marks the label that shows tube reload status.
#[derive(Component)]
struct TubeStatusLabel(crate::messages::TorpedoTube);

/// Marks the root node of the console tab bar, shown when the local player
/// holds 2+ consoles while in-game.
#[derive(Component)]
pub struct TabBarRoot;

/// Marks a single tab button in the tab bar; carries the console it selects.
#[derive(Component)]
struct TabButton(Console);

// ── Plugin ─────────────────────────────────────────────────────────

pub struct ClientAppPlugin;

impl Plugin for ClientAppPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(
                    10.0 / 255.0,
                    10.0 / 255.0,
                    26.0 / 255.0,
                )))
            .init_resource::<LobbyState>()
            .init_resource::<ClientSimState>()
            .init_resource::<LocalPlayerToken>()
            .init_resource::<ActiveConsole>()
            .init_resource::<SelectedTube>()
            .insert_resource(HelmJoystickState::default())
            .insert_resource(HelmTickTimer(Timer::from_seconds(0.1, TimerMode::Repeating)))
            .add_message::<InboundServerMessage>()
            .add_message::<OutboundClientMessage>()
            .add_systems(Startup, (setup_lobby_ui, setup_captain_ui, setup_helm_ui, setup_science_ui, setup_weapons_ui, setup_repair_ui, setup_power_ui, setup_tab_bar_ui))
            .add_systems(
                Update,
                (
                    (
                        apply_inbound_messages,
                        rebuild_lobby_ui_on_change,
                        toggle_lobby_visibility_on_phase,
                        toggle_captain_panel_visibility,
                        refresh_view_dir_highlights,
                        refresh_red_alert_button,
                    ),
                    (
                        handle_station_button_press,
                        handle_engage_button_press,
                        handle_view_dir_button_press,
                        handle_red_alert_button_press,
                    ),
                    (
                        toggle_helm_panel_visibility,
                        helm_resend_tick,
                        refresh_helm_knob_position,
                        refresh_helm_readout,
                        handle_on_screen_button_press,
                        handle_repair_button_press,
                        refresh_repair_button,
                        refresh_repair_icon,
                        draw_helm_radar,
                    ),
                    (
                        toggle_science_panel_visibility,
                        toggle_weapons_panel_visibility,
                        toggle_repair_panel_visibility,
                        toggle_power_panel_visibility,
                    ),
                    (
                        handle_fire_phaser_button_press,
                        handle_phaser_mode_toggle_press,
                        handle_torpedo_tube_button_press,
                        handle_fire_torpedo_button_press,
                    ),
                    (
                        refresh_weapons_panel,
                        refresh_torpedo_ui,
                    ),
                    (
                        refresh_repair_panel,
                        handle_repair_shape_button_press,
                    ),
                    (
                        refresh_power_panel,
                        handle_increase_power,
                        handle_decrease_power,
                    ),
                    (
                        rebuild_tab_bar,
                        handle_tab_button_press,
                    ),
                ),
            );
    }
}

/// 10Hz resend timer for the helm joystick.
#[derive(Resource)]
struct HelmTickTimer(Timer);

// ── Setup ──────────────────────────────────────────────────────────

fn setup_lobby_ui(mut commands: Commands) {
    // 2D camera for UI rendering. `IsDefaultUiCamera` marks this as the
    // target for UI roots that don't carry an explicit `UiTargetCamera`,
    // which Bevy 0.18 requires for text glyph extraction to resolve a
    // camera deterministically.
    commands.spawn((Camera2d, IsDefaultUiCamera));

    commands
        .spawn((
            LobbyRoot,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::FlexStart,
                padding: UiRect::all(Val::Px(16.0)),
                row_gap: Val::Px(8.0),
                ..default()
            },
        ))
        .with_children(|root| {
            root.spawn((
                Text::new("Lobby"),
                TextFont { font_size: 22.0, ..default() },
                TextColor(Color::srgb(0.53, 0.67, 1.0)),
            ));

            root.spawn((
                ConsoleListRoot,
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    ..default()
                },
            ));

            root.spawn((
                EngageButton,
                Button,
                Node {
                    padding: UiRect::all(Val::Px(10.0)),
                    margin: UiRect::top(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgb(0.13, 0.13, 0.27)),
                Visibility::Hidden,
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("Engage"),
                    TextFont { font_size: 16.0, ..default() },
                    TextColor(Color::srgb(0.93, 0.93, 1.0)),
                ));
            });

            root.spawn((
                PlayerListRoot,
                Node {
                    flex_direction: FlexDirection::Column,
                    margin: UiRect::top(Val::Px(12.0)),
                    row_gap: Val::Px(2.0),
                    ..default()
                },
            ));
        });
}

// ── Systems ────────────────────────────────────────────────────────

fn apply_inbound_messages(
    mut reader: MessageReader<InboundServerMessage>,
    mut lobby: ResMut<LobbyState>,
    mut sim: ResMut<ClientSimState>,
    token: Res<LocalPlayerToken>,
    mut active: ResMut<ActiveConsole>,
) {
    for ev in reader.read() {
        // Before updating lobby state, intercept StationAssigned for the
        // local player to reconcile the active-console tab.
        if let ServerMessage::StationAssigned { token: t, consoles, .. } = &ev.0 {
            if t == &token.0 && !consoles.is_empty() {
                active.0 = Some(reconcile_active_console(active.0.clone(), consoles));
            } else if t == &token.0 && consoles.is_empty() {
                // Spectator — clear active console.
                active.0 = None;
            }
        }
        lobby.apply(&ev.0);
        sim.apply(&ev.0);
    }
}

fn toggle_lobby_visibility_on_phase(
    state: Res<LobbyState>,
    mut roots: Query<&mut Visibility, With<LobbyRoot>>,
) {
    if !state.is_changed() {
        return;
    }
    let in_lobby = state.phase == GamePhase::Lobby;
    for mut vis in roots.iter_mut() {
        *vis = if in_lobby { Visibility::Visible } else { Visibility::Hidden };
    }
}

fn rebuild_lobby_ui_on_change(
    mut commands: Commands,
    state: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    console_root: Query<Entity, With<ConsoleListRoot>>,
    player_root: Query<Entity, With<PlayerListRoot>>,
    children_q: Query<&Children>,
    mut engage: Query<&mut Visibility, With<EngageButton>>,
) {
    if !state.is_changed() && !token.is_changed() {
        return;
    }
    let view = LobbyView::new(&state, &token.0);

    // Console buttons.
    if let Ok(root) = console_root.single() {
        if let Ok(children) = children_q.get(root) {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }
        commands.entity(root).with_children(|parent| {
            for slot in view.station_slots() {
                spawn_station_row(parent, &slot);
            }
        });
    }

    // Player list lines.
    if let Ok(root) = player_root.single() {
        if let Ok(children) = children_q.get(root) {
            for child in children.iter() {
                commands.entity(child).despawn();
            }
        }
        commands.entity(root).with_children(|parent| {
            for p in &state.players {
                let mark = if p.token == token.0 { "▶ " } else { "• " };
                let consoles = if p.consoles.is_empty() {
                    String::new()
                } else {
                    let names: Vec<String> =
                        p.consoles.iter().map(|c| {
                            let preset = state.complexity.get(c)
                                .map(|s| format!(" ({})", s))
                                .unwrap_or_default();
                            format!("{}{preset}", c.display_name())
                        }).collect();
                    format!(" — {}", names.join(", "))
                };
                parent.spawn((
                    Text::new(format!("{mark}{}{consoles}", p.name)),
                    TextFont { font_size: 13.0, ..default() },
                    TextColor(Color::srgb(0.6, 0.7, 0.73)),
                ));
            }
        });
    }

    // Engage visibility — only show when captain, in lobby, and all stations filled.
    if let Ok(mut vis) = engage.single_mut() {
        *vis = if view.is_captain() && state.phase == GamePhase::Lobby && view.all_stations_filled() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

fn spawn_station_row(parent: &mut ChildSpawnerCommands, slot: &StationSlot) {
    let (label, station_for_click, bg, fg) = match slot {
        StationSlot::Available { station, description, preset_names } => {
            let presets = if preset_names.is_empty() { String::new() } else {
                format!(" [{}]", preset_names.join(", "))
            };
            (format!("{}: available — {}{}", station, description, presets),
            Some(station.clone()),
            Color::srgb(0.13, 0.13, 0.27),
            Color::srgb(0.93, 0.93, 1.0))
        }
        StationSlot::Occupied { station, description, holder_name, preset_names } => {
            let presets = if preset_names.is_empty() { String::new() } else {
                format!(" [{}]", preset_names.join(", "))
            };
            (format!("{}: {} — {}{}", station, holder_name, description, presets),
            None,
            Color::srgb(0.07, 0.07, 0.10),
            Color::srgb(0.42, 0.49, 0.55))
        }
        StationSlot::Mine { station, description, preset_names } => {
            let presets = if preset_names.is_empty() { String::new() } else {
                format!(" [{}]", preset_names.join(", "))
            };
            (format!("{}: Mine — {} (leave){}", station, description, presets),
            None,
            Color::srgb(0.20, 0.24, 0.40),
            Color::srgb(0.55, 0.70, 1.0))
        }
        StationSlot::Spectator { player_name } => (
            format!("{} — (spectating)", player_name),
            None,
            Color::srgb(0.07, 0.07, 0.10),
            Color::srgb(0.42, 0.49, 0.55),
        ),
    };

    let mut row = parent.spawn((
        Node {
            padding: UiRect::all(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(bg),
    ));
    if let Some(s) = station_for_click {
        row.insert((Button, StationButton(s)));
    }
    row.with_children(|inner| {
        inner.spawn((
            Text::new(label),
            TextFont { font_size: 14.0, ..default() },
            TextColor(fg),
        ));
    });
}

fn handle_station_button_press(
    mut interactions: Query<
        (&Interaction, &StationButton),
        (Changed<Interaction>, With<Button>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, StationButton(s)) in interactions.iter_mut() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let slot = StationSlot::Available { station: s.clone(), description: String::new(), preset_names: vec![] };
        if let Some(msg) = message_for_station_slot_click(&slot) {
            outbound.write(OutboundClientMessage(msg));
        }
    }
}

fn handle_engage_button_press(
    mut interactions: Query<
        &Interaction,
        (Changed<Interaction>, With<Button>, With<EngageButton>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(engage_message()));
        }
    }
}

// ── Captain console UI ─────────────────────────────────────────────

/// Background colour for an inactive direction button in the cross.
const VIEW_BTN_BG_INACTIVE: Color = Color::srgb(0.13, 0.13, 0.27);
/// Background colour for the currently active direction button.
const VIEW_BTN_BG_ACTIVE:   Color = Color::srgb(0.20, 0.24, 0.40);
/// Background for the Red Alert toggle when alert is OFF.
const RED_ALERT_BG_OFF: Color = Color::srgb(0.13, 0.13, 0.27);
/// Background for the Red Alert toggle when alert is ON (deep red).
const RED_ALERT_BG_ON:  Color = Color::srgb(0.40, 0.0, 0.0);

fn setup_captain_ui(_commands: Commands) {
    // Replaced by phone_border::captain::CaptainPanelPlugin.
    // The old captain UI (direction grid + red alert text toggle) is no
    // longer spawned here — the phone-border captain panel plugin takes
    // over all captain panel rendering.
}

fn toggle_captain_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<CaptainPanel>>,
) {
    if !lobby.is_changed() && !token.is_changed() && !active.is_changed() {
        return;
    }
    let view = LobbyView::new(&lobby, &token.0);
    let holds_captain = lobby.phase == GamePhase::InProgress && view.is_captain();
    // When the player holds multiple consoles, only show this panel when
    // the tab is explicitly set to CaptainChair (or unset with only 1 console).
    let tab_active = match &active.0 {
        Some(c) => *c == Console::CaptainChair,
        None => true,
    };
    let visible = holds_captain && tab_active;
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

fn refresh_view_dir_highlights(
    sim: Res<ClientSimState>,
    mut buttons: Query<(&ViewDirButton, &mut BackgroundColor)>,
) {
    if !sim.is_changed() {
        return;
    }
    for (ViewDirButton(direction), mut bg) in buttons.iter_mut() {
        let active = sim.is_active_camera_direction(direction);
        bg.0 = if active { VIEW_BTN_BG_ACTIVE } else { VIEW_BTN_BG_INACTIVE };
    }
}

fn refresh_red_alert_button(
    sim: Res<ClientSimState>,
    mut button: Query<(Entity, &mut BackgroundColor), With<RedAlertButton>>,
    children_q: Query<&Children>,
    mut labels: Query<&mut Text, With<RedAlertLabel>>,
) {
    if !sim.is_changed() {
        return;
    }
    let Ok((button_entity, mut bg)) = button.single_mut() else { return };
    bg.0 = if sim.red_alert { RED_ALERT_BG_ON } else { RED_ALERT_BG_OFF };
    if let Ok(children) = children_q.get(button_entity) {
        for child in children.iter() {
            if let Ok(mut text) = labels.get_mut(child) {
                **text = if sim.red_alert {
                    "Red Alert: ON".to_string()
                } else {
                    "Red Alert: OFF".to_string()
                };
            }
        }
    }
}

fn handle_view_dir_button_press(
    mut interactions: Query<
        (&Interaction, &ViewDirButton),
        (Changed<Interaction>, With<Button>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, ViewDirButton(direction)) in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(message_for_direction_press(direction.clone())));
        }
    }
}

fn handle_red_alert_button_press(
    mut interactions: Query<
        &Interaction,
        (Changed<Interaction>, With<Button>, With<RedAlertButton>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(red_alert_toggle_message()));
        }
    }
}

// ── Helm console UI ────────────────────────────────────────────────

/// Diameter of the joystick pad in logical pixels. The knob is constrained
/// to a circle whose radius is `(PAD_SIZE / 2) - HELM_KNOB_RADIUS - 2`,
/// matching the JS contract.
const HELM_PAD_SIZE: f32 = 200.0;
/// Radius of the knob disc, in pixels.
const HELM_KNOB_RADIUS: f32 = 24.0;


fn setup_helm_ui(_commands: Commands) {
    // Replaced by phone_border::helm::HelmPanelPlugin.
    // The old UI (radar gizmos + plain thumbstick) is no longer spawned
    // here — the phone-border compass-ring radar + polished thumbstick
    // plugin takes over all helm panel rendering.
}

fn setup_science_ui(mut commands: Commands) {
    commands.spawn((
        SciencePanel,
        Node {
            position_type: PositionType::Absolute,
            left:   Val::Px(0.0),
            top:    Val::Px(0.0),
            right:  Val::Px(0.0),
            bottom: Val::Px(0.0),
            flex_direction: FlexDirection::Column,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            row_gap: Val::Px(8.0),
            ..default()
        },
        Visibility::Hidden,
    ))
    .with_children(|panel| {
        panel.spawn((
            Text::new("Science Console"),
            TextFont { font_size: 32.0, ..default() },
            TextColor(Color::srgb(0.8, 0.8, 1.0)),
        ));
        // Repair icon label — shows when a breakdown or decoy icon
        // targets this console.
        panel.spawn((
            RepairIconLabel,
            Text::new(""),
            TextFont { font_size: 14.0, ..default() },
            TextColor(Color::srgb(0.8, 0.5, 0.2)),
        ));
    });
}

fn setup_repair_ui(mut commands: Commands) {
    commands
        .spawn((
            RepairPanel,
            Node {
                position_type: PositionType::Absolute,
                left:   Val::Px(0.0),
                top:    Val::Px(0.0),
                right:  Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(12.0),
                padding: UiRect::all(Val::Px(24.0)),
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Repair Console"),
                TextFont { font_size: 24.0, ..default() },
                TextColor(Color::srgb(0.3, 1.0, 0.5)),
            ));

            // Breakdown row
            panel.spawn((
                RepairBreakdownLabel,
                Text::new("All Systems Nominal"),
                TextFont { font_size: 16.0, ..default() },
                TextColor(Color::srgb(0.8, 0.8, 0.3)),
            ));

            // Shape buttons row
            panel.spawn((
                RepairShapeButtonRoot,
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(8.0),
                    ..default()
                },
            )).with_children(|row| {
                for (shape, label) in [
                    (Shape::Square, "SQUARE"),
                    (Shape::Triangle, "TRIANGLE"),
                    (Shape::Circle, "CIRCLE"),
                ] {
                    row.spawn((
                        RepairShapeButton(shape),
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(16.0), Val::Px(12.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.10, 0.25, 0.15)),
                    )).with_children(|btn| {
                        btn.spawn((
                            Text::new(label),
                            TextFont { font_size: 16.0, ..default() },
                            TextColor(Color::srgb(0.5, 1.0, 0.7)),
                        ));
                    });
                }
            });

            // Team rows
            for i in 0..3 {
                panel.spawn((
                    RepairTeamRow(i),
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        width: Val::Percent(80.0),
                        height: Val::Px(36.0),
                        position_type: PositionType::Relative,
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.05, 0.10, 0.20)),
                )).with_children(|row| {
                    // Progress bar fill
                    row.spawn((
                        RepairTeamFill(i),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Percent(0.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.10, 0.20, 0.60)),
                    ));
                    // Status text
                    row.spawn((
                        RepairTeamStatusText(i),
                        Text::new("Idle"),
                        TextFont { font_size: 14.0, ..default() },
                        TextColor(Color::srgb(0.6, 0.8, 1.0)),
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(12.0),
                            top: Val::Px(0.0),
                            bottom: Val::Px(0.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ));
                });
            }
        });
}

fn toggle_repair_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<RepairPanel>>,
) {
    if !lobby.is_changed() && !token.is_changed() && !active.is_changed() {
        return;
    }
    let view = LobbyView::new(&lobby, &token.0);
    let holds_repair = lobby.phase == GamePhase::InProgress
        && view.my_consoles().contains(&Console::Repair);
    let tab_active = match &active.0 {
        Some(c) => *c == Console::Repair,
        None => true,
    };
    let visible = holds_repair && tab_active;
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

/// Refresh the repair panel: breakdown text, shape button states, team status.
fn refresh_repair_panel(
    sim: Res<ClientSimState>,
    mut breakdown_label: Query<&mut Text, (With<RepairBreakdownLabel>, Without<RepairTeamStatusText>)>,
    mut shape_btn_bg: Query<(&mut BackgroundColor, &RepairShapeButton), Without<RepairTeamFill>>,
    mut team_fill: Query<(&mut Node, &mut BackgroundColor, &RepairTeamFill), Without<RepairShapeButton>>,
    mut team_status: Query<(&mut Text, &mut TextColor, &RepairTeamStatusText), Without<RepairBreakdownLabel>>,
) {
    if !sim.is_changed() {
        return;
    }

    // Update breakdown label
    for mut text in breakdown_label.iter_mut() {
        **text = match &sim.current_breakdown {
            Some((console, shape)) => format!("{} — {:?}", console.display_name(), shape),
            None => "All Systems Nominal".to_string(),
        };
    }

    // Determine if all three teams are busy (no Idle slot)
    let all_busy = sim.repair_teams.iter().all(|t| !matches!(t, crate::messages::TeamSlot::Idle));

    // Update shape button backgrounds based on busy state
    for (mut bg, _) in shape_btn_bg.iter_mut() {
        *bg = if all_busy {
            BackgroundColor(Color::srgb(0.08, 0.12, 0.10))
        } else {
            BackgroundColor(Color::srgb(0.15, 0.35, 0.20))
        };
    }

    // Update team progress bars (width + color) and status text
    for (mut node, mut fill_bg, fill) in team_fill.iter_mut() {
        let idx = fill.0;
        if idx >= sim.repair_teams.len() {
            continue;
        }
        let slot = &sim.repair_teams[idx];
        let (pct, color) = match slot {
            crate::messages::TeamSlot::Idle => (0.0, Color::srgb(0.10, 0.20, 0.60)),
            crate::messages::TeamSlot::Repairing { progress } => {
                ((progress * 100.0).clamp(0.0, 100.0), Color::srgb(0.10, 0.70, 0.20))
            }
            crate::messages::TeamSlot::Cooldown { progress } => {
                ((progress * 100.0).clamp(0.0, 100.0), Color::srgb(0.70, 0.15, 0.15))
            }
        };
        node.width = Val::Percent(pct);
        fill_bg.0 = color;
    }

    // Update team status text
    for (mut text, mut color, status) in team_status.iter_mut() {
        let idx = status.0;
        if idx >= sim.repair_teams.len() {
            continue;
        }
        let slot = &sim.repair_teams[idx];
        match slot {
            crate::messages::TeamSlot::Idle => {
                **text = "Idle".to_string();
                *color = TextColor(Color::srgb(0.5, 0.7, 1.0));
            }
            crate::messages::TeamSlot::Repairing { progress } => {
                **text = format!("Repairing {:.0}%", progress * 100.0);
                *color = TextColor(Color::srgb(0.3, 1.0, 0.3));
            }
            crate::messages::TeamSlot::Cooldown { progress } => {
                **text = format!("Cooldown {:.0}%", (1.0 - progress) * 100.0);
                *color = TextColor(Color::srgb(1.0, 0.4, 0.4));
            }
        }
    }
}

/// Handle shape button presses on the Repair console.
fn handle_repair_shape_button_press(
    interactions: Query<(&Interaction, &RepairShapeButton), Changed<Interaction>>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, shape_btn) in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        outbound.write(OutboundClientMessage(ClientMessage::Repair { shape: shape_btn.0 }));
    }
}

fn toggle_science_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<SciencePanel>>,
) {
    if !lobby.is_changed() && !token.is_changed() && !active.is_changed() {
        return;
    }
    let view = LobbyView::new(&lobby, &token.0);
    let holds_science = lobby.phase == GamePhase::InProgress
        && view.my_consoles().contains(&Console::Science);
    let tab_active = match &active.0 {
        Some(c) => *c == Console::Science,
        None => true,
    };
    let visible = holds_science && tab_active;
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

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
    let view = LobbyView::new(&lobby, &token.0);
    let holds_helm = lobby.phase == GamePhase::InProgress && view.is_helm();
    // When the player holds multiple consoles, only show this panel when
    // the tab is explicitly set to Helm (or unset with only 1 console).
    let tab_active = match &active.0 {
        Some(c) => *c == Console::Helm,
        None => true,
    };
    let visible = holds_helm && tab_active;
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

// ── Helm radar drawing ─────────────────────────────────────────────

/// Outer ring colour for the helm radar.
const RADAR_OUTER_RING_COLOR: Color = Color::srgb(0.55, 0.70, 1.0);
/// Mid ring colour (drawn at `RADAR_MID_RING / RADAR_RANGE` of the outer
/// radius).
const RADAR_MID_RING_COLOR:   Color = Color::srgb(0.30, 0.40, 0.65);
/// Asteroid blip colour.
const RADAR_ASTEROID_COLOR:   Color = Color::srgb(0.85, 0.75, 0.45);
/// Ship triangle colour (always points "up" since the radar is
/// ship-aligned).
const RADAR_SHIP_COLOR:       Color = Color::srgb(0.95, 0.95, 1.0);

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
    let viewport_w = window.width();   // logical pixels
    let viewport_h = window.height();  // logical pixels

    // In Bevy 0.18, GlobalTransform::translation() for UI nodes returns the
    // node centre in logical pixels (+y up). Shift origin from top-left to
    // screen-centre and flip y for Camera2d world space.
    let node_size = node.size();
    let node_centre_screen = gt.translation().truncate();
    let centre_world_x = node_centre_screen.x - viewport_w / 2.0;
    let centre_world_y = viewport_h / 2.0 - node_centre_screen.y;
    let centre = Vec2::new(centre_world_x, centre_world_y);

    let radius = node_size.x.min(node_size.y) * 0.5;
    if radius <= 0.0 {
        return;
    }

    // Outer ring represents 1.5x the helm radar range; everything inside is scaled down.
    const ZOOM: f32 = 1.5;
    gizmos.circle_2d(centre, radius, RADAR_OUTER_RING_COLOR);
    let helm_range = crate::client_sim::helm_radar_config().range;
    let mid_ratio = crate::radar::RADAR_MID_RING / helm_range;
    gizmos.circle_2d(centre, radius * mid_ratio / ZOOM, RADAR_MID_RING_COLOR);

    // Asteroids — use the unified helm radar view (RadarConfig-filtered).
    let helm_view = crate::client_sim::compute_helm_radar_view(&sim);
    for dot in &helm_view.dots {
        let pos = centre + Vec2::new(dot.radar_x * radius / ZOOM, dot.radar_y * radius / ZOOM);
        let pix_radius = (dot.scaled_radius * radius / ZOOM).max(2.0);
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

/// Update repair icon label on every frame where `ClientSimState.repair_icon` changes.
fn refresh_repair_icon(
    sim: Res<ClientSimState>,
    mut labels: Query<&mut Text, With<RepairIconLabel>>,
) {
    if !sim.is_changed() {
        return;
    }
    let text = match sim.repair_icon {
        Some(Shape::Square) => "■ REPAIR",
        Some(Shape::Triangle) => "▲ REPAIR",
        Some(Shape::Circle) => "● REPAIR",
        None => "",
    };
    for mut label in labels.iter_mut() {
        **label = text.to_string();
    }
}

fn handle_repair_button_press(
    mut interactions: Query<
        (&Interaction, &mut BackgroundColor),
        (Changed<Interaction>, With<Button>, With<RepairButton>),
    >,
    sim: Res<ClientSimState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, _bg) in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            // Suppress if already repairing or under penalty cooldown.
            if sim.repair_in_progress || sim.repair_penalty {
                continue;
            }
            outbound.write(OutboundClientMessage(crate::client_sim::repair_message()));
        }
    }
}

fn refresh_repair_button(
    time: Res<Time>,
    sim: Res<ClientSimState>,
    mut button: Query<&mut BackgroundColor, (With<RepairButton>, Without<RepairButtonLabel>)>,
    mut label: Query<(&mut Text, &mut TextColor), With<RepairButtonLabel>>,
) {
    // Always update while penalized (drives flash animation); otherwise
    // skip when nothing has changed to avoid unnecessary UI work.
    if !sim.is_changed() && !sim.repair_penalty {
        return;
    }
    for mut bg in button.iter_mut() {
        *bg = if sim.repair_penalty {
            // Flash between bright red and dark red at ~3 Hz.
            let flash = (time.elapsed_secs() * 3.0).floor() as i32 % 2 == 0;
            if flash {
                BackgroundColor(Color::srgb(0.70, 0.05, 0.05))
            } else {
                BackgroundColor(Color::srgb(0.20, 0.02, 0.02))
            }
        } else if sim.repair_in_progress {
            BackgroundColor(Color::srgb(0.05, 0.30, 0.05))
        } else {
            BackgroundColor(Color::srgb(0.13, 0.27, 0.13))
        };
    }
    for (mut text, mut color) in label.iter_mut() {
        if sim.repair_penalty {
            **text = format!("COOLDOWN {:.0}s", sim.repair_cooldown_secs);
            *color = TextColor(Color::srgb(1.0, 0.3, 0.3));
        } else if sim.repair_in_progress {
            **text = format!("REPAIRING {:.0}s", sim.repair_cooldown_secs);
            *color = TextColor(Color::srgb(0.5, 1.0, 0.5));
        } else {
            **text = "REPAIR".to_string();
            *color = TextColor(Color::srgb(0.5, 1.0, 0.5));
        }
    }
}

// ── Weapons panel ──────────────────────────────────────────────────

fn setup_weapons_ui(mut commands: Commands) {
    commands
        .spawn((
            WeaponsPanel,
            Node {
                position_type: PositionType::Absolute,
                left:   Val::Px(0.0),
                top:    Val::Px(0.0),
                right:  Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(16.0),
                padding: UiRect::all(Val::Px(24.0)),
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Weapons Console"),
                TextFont { font_size: 24.0, ..default() },
                TextColor(Color::srgb(1.0, 0.5, 0.2)),
            ));

            // Torpedo count label
            panel.spawn((
                TorpedoCountLabel,
                Text::new("Torpedoes: 10"),
                TextFont { font_size: 16.0, ..default() },
                TextColor(Color::srgb(0.8, 0.8, 0.2)),
            ));

            // Torpedo tube selection row
            panel.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(8.0),
                    ..default()
                },
            )).with_children(|row| {
                for (tube, label) in [
                    (crate::messages::TorpedoTube::ForePort, "FWD PORT"),
                    (crate::messages::TorpedoTube::ForeStarboard, "FWD STBD"),
                    (crate::messages::TorpedoTube::Aft, "AFT"),
                ] {
                    row.spawn((
                        TorpedoTubeButton(tube),
                        Button,
                        Node {
                            padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(0.10, 0.20, 0.30)),
                    )).with_children(|btn| {
                        btn.spawn((
                            Text::new(label),
                            TextFont { font_size: 14.0, ..default() },
                            TextColor(Color::srgb(0.6, 0.8, 1.0)),
                        ));
                    });
                }
            });

            // Tube status labels row
            panel.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(8.0),
                    ..default()
                },
            )).with_children(|row| {
                for tube in [
                    crate::messages::TorpedoTube::ForePort,
                    crate::messages::TorpedoTube::ForeStarboard,
                    crate::messages::TorpedoTube::Aft,
                ] {
                    row.spawn((
                        TubeStatusLabel(tube),
                        Text::new("LOADED"),
                        TextFont { font_size: 12.0, ..default() },
                        TextColor(Color::srgb(0.3, 1.0, 0.3)),
                        Node {
                            min_width: Val::Px(70.0),
                            ..default()
                        },
                    ));
                }
            });

            // Fire torpedo button
            panel
                .spawn((
                    FireTorpedoButton,
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(32.0), Val::Px(16.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.10, 0.30, 0.10)),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        FireTorpedoLabel,
                        Text::new("FIRE TORPEDO"),
                        TextFont { font_size: 22.0, ..default() },
                        TextColor(Color::srgb(0.3, 1.0, 0.3)),
                    ));
                });

            // Phaser mode toggle button
            panel
                .spawn((
                    PhaserModeButton,
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(24.0), Val::Px(12.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.15, 0.15, 0.35)),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        PhaserModeLabel,
                        Text::new("Mode: AUTO"),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::srgb(0.7, 0.7, 1.0)),
                    ));
                });

            // Fire phasers button
            panel
                .spawn((
                    FirePhaserButton,
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(32.0), Val::Px(16.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(0.40, 0.10, 0.10)),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        FirePhaserLabel,
                        Text::new("FIRE PHASERS"),
                        TextFont { font_size: 22.0, ..default() },
                        TextColor(Color::srgb(1.0, 0.5, 0.2)),
                    ));
                });

            // Repair icon label — shows when a breakdown or decoy icon
            // targets this console.
            panel.spawn((
                RepairIconLabel,
                Text::new(""),
                TextFont { font_size: 14.0, ..default() },
                TextColor(Color::srgb(0.8, 0.5, 0.2)),
            ));
        });
}

fn toggle_weapons_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<WeaponsPanel>>,
) {
    if !lobby.is_changed() && !token.is_changed() && !active.is_changed() {
        return;
    }
    let view = LobbyView::new(&lobby, &token.0);
    let holds_tactical = lobby.phase == GamePhase::InProgress
        && view.my_consoles().contains(&Console::Tactical);
    let tab_active = match &active.0 {
        Some(c) => *c == Console::Tactical,
        None => true,
    };
    let visible = holds_tactical && tab_active;
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

fn handle_fire_phaser_button_press(
    interactions: Query<&Interaction, (Changed<Interaction>, With<Button>, With<FirePhaserButton>)>,
    sim: Res<ClientSimState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Suppress if on cooldown.
        if sim.on_cooldown {
            continue;
        }
        outbound.write(OutboundClientMessage(fire_phaser_message()));
    }
}

fn handle_phaser_mode_toggle_press(
    interactions: Query<&Interaction, (Changed<Interaction>, With<Button>, With<PhaserModeButton>)>,
    sim: Res<ClientSimState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Toggle between Auto and Manual.
        let new_mode = match sim.phaser_mode {
            PhaserMode::Auto => PhaserMode::Manual,
            PhaserMode::Manual => PhaserMode::Auto,
        };
        outbound.write(OutboundClientMessage(set_phaser_mode_message(new_mode)));
    }
}

fn refresh_weapons_panel(
    sim: Res<ClientSimState>,
    mut fire_bg: Query<&mut BackgroundColor, (With<FirePhaserButton>, Without<PhaserModeButton>)>,
    mut fire_label: Query<(&mut Text, &mut TextColor), With<FirePhaserLabel>>,
    mut mode_label: Query<(&mut Text, &mut TextColor), (With<PhaserModeLabel>, Without<FirePhaserLabel>)>,
) {
    if !sim.is_changed() {
        return;
    }
    // Update fire button appearance.
    for mut bg in fire_bg.iter_mut() {
        *bg = if sim.on_cooldown {
            BackgroundColor(Color::srgb(0.20, 0.05, 0.05))
        } else if sim.fire_ready {
            BackgroundColor(Color::srgb(0.60, 0.10, 0.10))
        } else {
            BackgroundColor(Color::srgb(0.30, 0.08, 0.08))
        };
    }
    for (mut text, mut color) in fire_label.iter_mut() {
        if sim.on_cooldown {
            **text = "COOLING DOWN".to_string();
            *color = TextColor(Color::srgb(0.5, 0.2, 0.2));
        } else {
            **text = "FIRE PHASERS".to_string();
            *color = if sim.fire_ready {
                TextColor(Color::srgb(1.0, 0.5, 0.2))
            } else {
                TextColor(Color::srgb(0.5, 0.3, 0.2))
            };
        }
    }
    // Update mode button label.
    for (mut text, _) in mode_label.iter_mut() {
        **text = match sim.phaser_mode {
            PhaserMode::Auto => "Mode: AUTO".to_string(),
            PhaserMode::Manual => "Mode: MANUAL".to_string(),
        };
    }
}

// ── Torpedo UI systems ─────────────────────────────────────────────

/// Handle torpedo tube selection button presses — update `SelectedTube`.
fn handle_torpedo_tube_button_press(
    interactions: Query<(&Interaction, &TorpedoTubeButton), Changed<Interaction>>,
    mut selected: ResMut<SelectedTube>,
) {
    for (interaction, tube_btn) in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Toggle: press same tube again to deselect.
        if selected.0 == Some(tube_btn.0) {
            selected.0 = None;
        } else {
            selected.0 = Some(tube_btn.0);
        }
    }
}

/// Handle the Fire Torpedo button press. Fires from the selected tube with
/// the current target as the homing target (if any).
fn handle_fire_torpedo_button_press(
    interactions: Query<&Interaction, (Changed<Interaction>, With<Button>, With<FireTorpedoButton>)>,
    selected: Res<SelectedTube>,
    sim: Res<ClientSimState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let Some(tube) = selected.0 else { continue };
        // Check the chosen tube is loaded.
        let loaded = match tube {
            crate::messages::TorpedoTube::ForePort => sim.fore_port_loaded,
            crate::messages::TorpedoTube::ForeStarboard => sim.fore_starboard_loaded,
            crate::messages::TorpedoTube::Aft => sim.aft_loaded,
        };
        if !loaded || sim.torpedo_count == 0 {
            continue;
        }
        outbound.write(OutboundClientMessage(fire_torpedo_message(tube, None)));
    }
}

/// Refresh the torpedo UI (count, tube status labels, fire button, tube selection highlights).
fn refresh_torpedo_ui(
    sim: Res<ClientSimState>,
    selected: Res<SelectedTube>,
    mut count_label: Query<&mut Text, With<TorpedoCountLabel>>,
    mut tube_status: Query<(&mut Text, &mut TextColor, &TubeStatusLabel), (Without<TorpedoCountLabel>, Without<FireTorpedoLabel>)>,
    mut fire_bg: Query<&mut BackgroundColor, With<FireTorpedoButton>>,
    mut fire_label: Query<(&mut Text, &mut TextColor), (With<FireTorpedoLabel>, Without<TorpedoCountLabel>, Without<TubeStatusLabel>)>,
    mut tube_btn_bg: Query<(&mut BackgroundColor, &TorpedoTubeButton), Without<FireTorpedoButton>>,
) {
    if !sim.is_changed() && !selected.is_changed() {
        return;
    }

    // Update torpedo count label.
    for mut text in count_label.iter_mut() {
        **text = format!("Torpedoes: {}", sim.torpedo_count);
    }

    // Update per-tube status labels.
    for (mut text, mut color, label) in tube_status.iter_mut() {
        let (loaded, reload_secs) = match label.0 {
            crate::messages::TorpedoTube::ForePort =>
                (sim.fore_port_loaded, sim.fore_port_reload_secs),
            crate::messages::TorpedoTube::ForeStarboard =>
                (sim.fore_starboard_loaded, sim.fore_starboard_reload_secs),
            crate::messages::TorpedoTube::Aft =>
                (sim.aft_loaded, sim.aft_reload_secs),
        };
        if loaded {
            **text = "LOADED".to_string();
            *color = TextColor(Color::srgb(0.3, 1.0, 0.3));
        } else {
            **text = format!("{:.0}s", reload_secs.ceil());
            *color = TextColor(Color::srgb(1.0, 0.6, 0.2));
        }
    }

    // Update tube selection button highlights.
    for (mut bg, tube_btn) in tube_btn_bg.iter_mut() {
        let is_selected = selected.0 == Some(tube_btn.0);
        *bg = if is_selected {
            BackgroundColor(Color::srgb(0.10, 0.50, 0.70))
        } else {
            BackgroundColor(Color::srgb(0.10, 0.20, 0.30))
        };
    }

    // Update Fire Torpedo button appearance.
    let tube_ready = selected.0.map(|t| match t {
        crate::messages::TorpedoTube::ForePort => sim.fore_port_loaded,
        crate::messages::TorpedoTube::ForeStarboard => sim.fore_starboard_loaded,
        crate::messages::TorpedoTube::Aft => sim.aft_loaded,
    }).unwrap_or(false);
    let can_fire = tube_ready && sim.torpedo_count > 0 && selected.0.is_some();

    for mut bg in fire_bg.iter_mut() {
        *bg = if can_fire {
            BackgroundColor(Color::srgb(0.10, 0.50, 0.10))
        } else {
            BackgroundColor(Color::srgb(0.05, 0.20, 0.05))
        };
    }
    for (mut text, mut color) in fire_label.iter_mut() {
        if selected.0.is_none() {
            **text = "SELECT TUBE".to_string();
            *color = TextColor(Color::srgb(0.5, 0.5, 0.5));
        } else if sim.torpedo_count == 0 {
            **text = "NO TORPEDOES".to_string();
            *color = TextColor(Color::srgb(0.6, 0.3, 0.3));
        } else if !tube_ready {
            **text = "TUBE LOADING".to_string();
            *color = TextColor(Color::srgb(0.8, 0.5, 0.2));
        } else {
            **text = "FIRE TORPEDO".to_string();
            *color = TextColor(Color::srgb(0.3, 1.0, 0.3));
        }
    }
}

// ── Power console UI ───────────────────────────────────────────────

const POWER_COL_INACTIVE: Color = Color::srgb(0.08, 0.08, 0.12);
const POWER_COL_LOCKED: Color = Color::srgb(0.06, 0.06, 0.08);
const POWER_INC_COLOR: Color = Color::srgb(0.10, 0.50, 0.30);
const POWER_INC_LOCKED: Color = Color::srgb(0.06, 0.06, 0.10);
const POWER_DEC_COLOR: Color = Color::srgb(0.50, 0.20, 0.10);
const POWER_DEC_LOCKED: Color = Color::srgb(0.06, 0.06, 0.10);
const POWER_BATTERY_BG: Color = Color::srgb(0.06, 0.06, 0.15);
const POWER_BATTERY_FILL: Color = Color::srgb(0.10, 0.60, 0.80);

fn setup_power_ui(mut commands: Commands) {
    commands
        .spawn((
            PowerPanel,
            Node {
                position_type: PositionType::Absolute,
                left:   Val::Px(0.0),
                top:    Val::Px(0.0),
                right:  Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(12.0),
                padding: UiRect::all(Val::Px(24.0)),
                ..default()
            },
            Visibility::Hidden,
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new("Power Console"),
                TextFont { font_size: 24.0, ..default() },
                TextColor(Color::srgb(0.3, 1.0, 0.8)),
            ));

            // Three power rows: Helm, Weapons, Science
            for (console, label) in [
                (Console::Helm, "Helm"),
                (Console::Tactical, "Weapons"),
                (Console::Science, "Science"),
            ] {
                panel.spawn((
                    PowerRow(console.clone()),
                    Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(12.0),
                        padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                        ..default()
                    },
                    BackgroundColor(POWER_COL_INACTIVE),
                )).with_children(|row| {
                    // Console name
                    row.spawn((
                        Text::new(label),
                        TextFont { font_size: 16.0, ..default() },
                        TextColor(Color::srgb(0.7, 0.9, 1.0)),
                        Node { width: Val::Px(80.0), ..default() },
                    ));
                    // Decrement button
                    row.spawn((
                        PowerDecButton(console.clone()),
                        Button,
                        Node {
                            width: Val::Px(36.0), height: Val::Px(36.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(POWER_DEC_COLOR),
                    )).with_children(|btn| {
                        btn.spawn((
                            Text::new("-"),
                            TextFont { font_size: 22.0, ..default() },
                            TextColor(Color::srgb(0.9, 0.9, 1.0)),
                        ));
                    });
                    // Level text
                    row.spawn((
                        PowerRowLevel(console.clone()),
                        Text::new("2"),
                        TextFont { font_size: 18.0, ..default() },
                        TextColor(Color::srgb(0.9, 0.9, 1.0)),
                        Node { min_width: Val::Px(24.0), justify_content: JustifyContent::Center, ..default() },
                    ));
                    // Increment button
                    row.spawn((
                        PowerIncButton(console.clone()),
                        Button,
                        Node {
                            width: Val::Px(36.0), height: Val::Px(36.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        BackgroundColor(POWER_INC_COLOR),
                    )).with_children(|btn| {
                        btn.spawn((
                            Text::new("+"),
                            TextFont { font_size: 22.0, ..default() },
                            TextColor(Color::srgb(0.9, 0.9, 1.0)),
                        ));
                    });
                });
            }

            // Battery bar section
            panel.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(4.0),
                    width: Val::Percent(80.0),
                    max_width: Val::Px(300.0),
                    ..default()
                },
            )).with_children(|battery_section| {
                // Battery bar background
                battery_section.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(16.0),
                        position_type: PositionType::Relative,
                        ..default()
                    },
                    BackgroundColor(POWER_BATTERY_BG),
                )).with_children(|bar_bg| {
                    bar_bg.spawn((
                        BatteryBar,
                        Node {
                            position_type: PositionType::Absolute,
                            left: Val::Px(0.0),
                            top: Val::Px(0.0),
                            width: Val::Percent(0.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                        BackgroundColor(POWER_BATTERY_FILL),
                    ));
                });
                // Battery percentage label
                battery_section.spawn((
                    BatteryLabel,
                    Text::new("Battery: 0%"),
                    TextFont { font_size: 14.0, ..default() },
                    TextColor(Color::srgb(0.5, 0.8, 1.0)),
                ));
            });
        });
}

fn toggle_power_panel_visibility(
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    mut panel: Query<&mut Visibility, With<PowerPanel>>,
) {
    if !lobby.is_changed() && !token.is_changed() && !active.is_changed() {
        return;
    }
    let view = LobbyView::new(&lobby, &token.0);
    let holds_power = lobby.phase == GamePhase::InProgress
        && view.my_consoles().contains(&Console::Power);
    let tab_active = match &active.0 {
        Some(c) => *c == Console::Power,
        None => true,
    };
    let visible = holds_power && tab_active;
    for mut vis in panel.iter_mut() {
        *vis = if visible { Visibility::Visible } else { Visibility::Hidden };
    }
}

/// Refresh the power panel: power levels, button enable/disable, battery bar, lock state.
fn refresh_power_panel(
    sim: Res<ClientSimState>,
    mut row_bg: Query<(&mut BackgroundColor, &PowerRow), (Without<PowerIncButton>, Without<PowerDecButton>)>,
    mut level_labels: Query<(&mut Text, &PowerRowLevel), Without<BatteryLabel>>,
    mut inc_buttons: Query<(&mut BackgroundColor, &PowerIncButton), (Without<PowerRow>, Without<PowerDecButton>)>,
    mut dec_buttons: Query<(&mut BackgroundColor, &PowerDecButton), (Without<PowerRow>, Without<PowerIncButton>)>,
    mut battery_bar: Query<&mut Node, With<BatteryBar>>,
    mut battery_label: Query<&mut Text, (With<BatteryLabel>, Without<PowerRowLevel>)>,
) {
    if !sim.is_changed() {
        return;
    }

    let locked = crate::client_sim::is_power_locked(&sim.power_state_payload);
    let battery_pct = crate::client_sim::battery_percentage(&sim.power_state_payload);

    // Update battery bar width and label
    for mut node in battery_bar.iter_mut() {
        node.width = Val::Percent(battery_pct);
    }
    for mut text in battery_label.iter_mut() {
        **text = format!("Battery: {:.0}%", battery_pct);
    }

    // Update each power row background + level text
    for (mut bg, row) in row_bg.iter_mut() {
        let _console = &row.0;
        bg.0 = if locked { POWER_COL_LOCKED } else { POWER_COL_INACTIVE };
    }

    // Update level labels, matching by console
    for (mut text, level_component) in level_labels.iter_mut() {
        let lvl = match level_component.0 {
            Console::Helm => sim.power_levels.0,
            Console::Tactical => sim.power_levels.1,
            Console::Science => sim.power_levels.2,
            _ => 0,
        };
        **text = format!("{}", lvl);
    }

    // Update increment buttons by matching their console
    for (mut bg, inc) in inc_buttons.iter_mut() {
        let can_inc = crate::client_sim::can_increase_power(
            &sim.power_levels, &inc.0, locked,
        );
        bg.0 = if can_inc { POWER_INC_COLOR } else { POWER_INC_LOCKED };
    }

    // Update decrement buttons by matching their console
    for (mut bg, dec) in dec_buttons.iter_mut() {
        let can_dec = crate::client_sim::can_decrease_power(
            &sim.power_levels, &dec.0, locked,
        );
        bg.0 = if can_dec { POWER_DEC_COLOR } else { POWER_DEC_LOCKED };
    }
}

/// Handle increment button presses on the Power console.
fn handle_increase_power(
    interactions: Query<(&Interaction, &PowerIncButton), Changed<Interaction>>,
    sim: Res<ClientSimState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, inc) in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let locked = crate::client_sim::is_power_locked(&sim.power_state_payload);
        if !crate::client_sim::can_increase_power(&sim.power_levels, &inc.0, locked) {
            continue;
        }
        outbound.write(OutboundClientMessage(
            crate::client_sim::increase_power_message(inc.0.clone()),
        ));
    }
}

/// Handle decrement button presses on the Power console.
fn handle_decrease_power(
    interactions: Query<(&Interaction, &PowerDecButton), Changed<Interaction>>,
    sim: Res<ClientSimState>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, dec) in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        let locked = crate::client_sim::is_power_locked(&sim.power_state_payload);
        if !crate::client_sim::can_decrease_power(&sim.power_levels, &dec.0, locked) {
            continue;
        }
        outbound.write(OutboundClientMessage(
            crate::client_sim::decrease_power_message(dec.0.clone()),
        ));
    }
}

// ── Tab Bar ────────────────────────────────────────────────────────

fn setup_tab_bar_ui(mut commands: Commands) {
    commands.spawn((
        TabBarRoot,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(0.0),
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            height: Val::Px(44.0),
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(4.0),
            padding: UiRect::all(Val::Px(4.0)),
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(Color::srgba(0.05, 0.05, 0.15, 0.92)),
        Visibility::Hidden,
    ));
}

/// Rebuilds the tab bar whenever the lobby / active-console state changes.
/// Spawns one button child per console in the local player's bundle.
fn rebuild_tab_bar(
    mut commands: Commands,
    lobby: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    active: Res<ActiveConsole>,
    tab_root_q: Query<Entity, With<TabBarRoot>>,
    mut tab_vis_q: Query<&mut Visibility, With<TabBarRoot>>,
    children_q: Query<&Children>,
) {
    if !lobby.is_changed() && !token.is_changed() && !active.is_changed() {
        return;
    }

    let Ok(root) = tab_root_q.single() else { return };

    let view = LobbyView::new(&lobby, &token.0);
    let my_consoles = view.my_consoles();
    let in_game = lobby.phase == GamePhase::InProgress;
    let show_tabs = in_game && my_consoles.len() >= 2;

    // Show/hide the bar.
    if let Ok(mut vis) = tab_vis_q.single_mut() {
        *vis = if show_tabs { Visibility::Visible } else { Visibility::Hidden };
    }

    // Rebuild children.
    if let Ok(children) = children_q.get(root) {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    if !show_tabs {
        return;
    }

    commands.entity(root).with_children(|parent| {
        for console in my_consoles {
            let is_active = active.0.as_ref() == Some(console);
            let bg = if is_active {
                Color::srgb(0.20, 0.40, 0.80)
            } else {
                Color::srgb(0.10, 0.15, 0.30)
            };
            let mut btn = parent.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(bg),
            ));
            btn.insert((Button, TabButton(console.clone())));
            btn.with_children(|inner| {
                inner.spawn((
                    Text::new(console.display_name()),
                    TextFont { font_size: 14.0, ..default() },
                    TextColor(Color::WHITE),
                ));
            });
        }
    });
}

/// Handles tab button presses by updating `ActiveConsole` directly (bypasses
/// JS `wasm_client_set_active_console` — the Bevy resource is the source of
/// truth; the bridge forwards the JS value only when JS calls that function).
fn handle_tab_button_press(
    mut interactions: Query<
        (&Interaction, &TabButton),
        (Changed<Interaction>, With<Button>),
    >,
    mut active: ResMut<ActiveConsole>,
) {
    for (interaction, TabButton(console)) in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            active.0 = Some(console.clone());
        }
    }
}
