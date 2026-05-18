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

use crate::client_lobby::{
    engage_message, message_for_station_slot_click, reconcile_active_console, StationSlot,
    LobbyState, LobbyView, LocalPlayerToken, ActiveConsole,
};
use crate::client_sim::ClientSimState;
use crate::client_complexity::{self, ComplexityStore};
use crate::client_elements::{
    handle_help_button_press, handle_help_overlay_dismiss, HideableElementRegistry,
};
use crate::messages::{ClientMessage, Console, GamePhase, ServerMessage};

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

/// Tracks whether the window is currently in landscape orientation.
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct LandscapeMode(pub bool);

/// Marks the root node of the lobby UI so it can be shown/hidden when
/// the phase changes, and reparented into the bezel content area.
#[derive(Component)]
pub struct LobbyRoot;

/// Marks the game-over overlay screen.
#[derive(Component)]
pub struct GameOverScreen;

/// Marks the text entity that displays the game-over reason.
#[derive(Component)]
pub struct GameOverReasonText;

/// Marks the container of the per-console buttons so it can be cleared
/// and rebuilt on every `LobbyState` change.
#[derive(Component)]
struct ConsoleListRoot;

/// Marks the container of the player list lines.
#[derive(Component)]
#[allow(dead_code)]
struct PlayerListRoot;

/// Marks the Engage button so we can toggle its visibility per captaincy.
#[derive(Component)]
struct EngageButton;

/// Marks one station-row button and remembers which station name it acts on.
#[derive(Component)]
struct StationButton(String);

/// Marks the scenario intro block (title + description) in the lobby.
#[derive(Component)]
struct ScenarioIntroBlock;

/// Marks the scenario title text inside the intro block.
#[derive(Component)]
struct ScenarioIntroTitle;

/// Marks the scenario description body text inside the intro block.
#[derive(Component)]
struct ScenarioIntroBody;

/// Marks the crew header node so it can be updated.
#[derive(Component)]
struct CrewHeader;

/// Marks the crew count "current" text node.
#[derive(Component)]
struct CrewCountCurrent;

/// Marks the crew count "max" text node.
#[derive(Component)]
struct CrewCountMax;

/// Marks the ready pill text node.
#[derive(Component)]
struct ReadyPill;

/// Marks the footer status text node.
#[derive(Component)]
struct FooterStatus;

/// Marks the Release button on a Mine station row.
#[derive(Component)]
struct ReleaseStationButton;

/// Marks the complexity segmented control container.
#[derive(Component)]
struct ComplexitySegControl;

/// Marks a complexity option button; carries the wire preset name ("Low" or "Std").
#[derive(Component)]
struct ComplexityOptionButton(String);

/// Marks the station detail panel root.
#[derive(Component)]
struct StationDetailPanel;

/// Marks the station detail title text.
#[derive(Component)]
struct StationDetailTitle;

/// Marks the consoles chip container inside the detail panel.
#[derive(Component)]
struct StationDetailConsoles;

/// Marks the root of the captain console UI (view selector + Red Alert);
/// shown only when the local player holds CaptainChair and the phase is
/// InProgress.
#[derive(Component)]
pub struct CaptainPanel;

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


/// Marks the root of the weapons console UI; shown only when the local
/// player holds Tactical and the phase is InProgress.
#[derive(Component)]
pub struct WeaponsPanel;

/// Marks the radar display node inside the weapons console (used by
/// `draw_weapons_radar` to locate where to draw gizmo blips).
#[derive(Component)]
pub struct WeaponsRadarPanel;

/// Marks the complexity preset pop-up overlay root.
#[derive(Component)]
pub struct ComplexityPopupRoot;

/// Marks a preset option button inside the pop-up or dropdown.
/// Carries the preset name as payload (e.g. "Low", "Std").
#[derive(Component)]
pub struct ComplexityPresetButton(pub String);

/// Marks the confirm button on the pop-up.
#[derive(Component)]
pub struct ComplexityPopupConfirm;

/// Marks the complexity dropdown row root.
#[derive(Component)]
pub struct ComplexityDropdownRoot;


/// Marks the root node of the console tab bar, shown when the local player
/// holds 2+ consoles while in-game.
#[derive(Component)]
pub struct TabBarRoot;

/// Marks a single tab button in the tab bar; carries the console it selects.
#[derive(Component)]
struct TabButton(Console);

/// Marks a UI element that can be hidden by complexity preset `hidden_elements`.
/// The string name must match an entry in the complexity TOML for this console.
#[derive(Component)]
pub struct HideableElement(pub String);

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
            .init_resource::<ComplexityStore>()
            .init_resource::<HideableElementRegistry>()
            .init_resource::<LandscapeMode>()
            .add_message::<InboundServerMessage>()
            .add_message::<OutboundClientMessage>()
            .add_systems(Startup, (setup_lobby_ui, detect_initial_orientation, setup_helm_ui, setup_tab_bar_ui))
                .add_systems(
                    Update,
                    (
                        (
                            apply_inbound_messages,
                            detect_orientation_change,
                            rebuild_lobby_ui_on_change,
                            refresh_engage_button,
                            refresh_crew_header,
                            update_scenario_intro,
                            refresh_footer_status,
                            refresh_station_detail,
                            toggle_lobby_visibility_on_phase,
                            toggle_game_over_visibility,
                        ),
                        (
                            handle_station_button_press,
                            handle_release_station_button_press,
                            handle_engage_button_press,
                            handle_complexity_option_press,
                        ),
                        (
                            handle_repair_button_press,
                            refresh_repair_button,
                        ),

                        (
                            rebuild_tab_bar,
                            handle_tab_button_press,
                        ),
                        (
                            refresh_complexity_ui,
                            handle_complexity_preset_press,
                            handle_complexity_popup_confirm,
                        ),
                        (
                            register_hideable_elements,
                            sync_complexity_hiding,
                        ),
                        (
                            handle_help_button_press,
                            handle_help_overlay_dismiss,
                        ),
                    ),
                );
    }
}

// ── Setup ──────────────────────────────────────────────────────────

// ── Lobby colour palette ───────────────────────────────────────────
const COL_GRAPHITE_DARK: Color = Color::srgb(0.078, 0.090, 0.110);
const COL_GRAPHITE:      Color = Color::srgb(0.133, 0.149, 0.176);
const COL_EDGE:          Color = Color::srgb(0.227, 0.251, 0.286);
const COL_SIGNAL:        Color = Color::srgb(0.373, 0.847, 0.910);
const COL_TEXT:          Color = Color::srgb(0.722, 0.753, 0.784);
const COL_TEXT_DIM:      Color = Color::srgb(0.416, 0.447, 0.482);
const COL_AMBER:         Color = Color::srgb(0.941, 0.627, 0.125);

fn setup_lobby_ui(mut commands: Commands, asset_server: Res<AssetServer>) {
    // 2D camera for UI rendering. `IsDefaultUiCamera` marks this as the
    // target for UI roots that don't carry an explicit `UiTargetCamera`,
    // which Bevy 0.18 requires for text glyph extraction to resolve a
    // camera deterministically.
    commands.spawn((Camera2d, IsDefaultUiCamera));

    let chakra = asset_server.load("fonts/ChakraPetch-SemiBold.ttf");
    let mono   = asset_server.load("fonts/JetBrainsMono-Regular.ttf");

    commands.spawn((
        LobbyRoot,
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(Val::Px(10.0)),
            row_gap: Val::Px(8.0),
            ..default()
        },
        BackgroundColor(Color::srgba(0.04, 0.047, 0.063, 1.0)),
        Visibility::Hidden,
    ))
    .with_children(|root| {
        // ── Crew header ──────────────────────────────────────────────
        root.spawn((
            CrewHeader,
            Node {
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                ..default()
            },
            BackgroundColor(COL_GRAPHITE_DARK),
        ))
        .with_children(|hdr| {
            // Ship name column
            hdr.spawn(Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(2.0), ..default() })
            .with_children(|ship| {
                ship.spawn((
                    Text::new("PHOENIX"),
                    TextFont { font: chakra.clone(), font_size: 11.0, ..default() },
                    TextColor(COL_TEXT),
                ));
                ship.spawn((
                    Text::new("PRE-FLIGHT"),
                    TextFont { font: mono.clone(), font_size: 8.0, ..default() },
                    TextColor(COL_TEXT_DIM),
                ));
            });
            // Right side: crew count + ready pill
            hdr.spawn(Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: Val::Px(10.0), ..default() })
            .with_children(|right| {
                // Crew count
                right.spawn(Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Baseline, column_gap: Val::Px(2.0), ..default() })
                .with_children(|cc| {
                    cc.spawn((
                        Text::new("CREW "),
                        TextFont { font: chakra.clone(), font_size: 8.0, ..default() },
                        TextColor(COL_TEXT_DIM),
                    ));
                    cc.spawn((
                        CrewCountCurrent,
                        Text::new("0"),
                        TextFont { font: mono.clone(), font_size: 16.0, ..default() },
                        TextColor(COL_SIGNAL),
                    ));
                    cc.spawn((
                        Text::new("/"),
                        TextFont { font: mono.clone(), font_size: 12.0, ..default() },
                        TextColor(COL_TEXT_DIM),
                    ));
                    cc.spawn((
                        CrewCountMax,
                        Text::new("0"),
                        TextFont { font: mono.clone(), font_size: 12.0, ..default() },
                        TextColor(COL_TEXT_DIM),
                    ));
                });
                // Ready pill
                right.spawn((
                    ReadyPill,
                    Node {
                        padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.941, 0.627, 0.125, 0.08)),
                ))
                .with_children(|pill| {
                    pill.spawn((
                        ReadyPillText,
                        Text::new("AWAITING"),
                        TextFont { font: chakra.clone(), font_size: 8.0, ..default() },
                        TextColor(COL_AMBER),
                    ));
                });
            });
        });

        // ── Scenario intro block (initially hidden) ────────────────
        root.spawn((
            ScenarioIntroBlock,
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(6.0),
                padding: UiRect::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.078, 0.090, 0.110, 0.4)),
            Visibility::Hidden,
        ))
        .with_children(|block| {
            block.spawn((
                ScenarioIntroTitle,
                Text::new(""),
                TextFont { font: chakra.clone(), font_size: 14.0, ..default() },
                TextColor(COL_SIGNAL),
            ));
            block.spawn((
                ScenarioIntroBody,
                Text::new(""),
                TextFont { font: mono.clone(), font_size: 10.0, ..default() },
                TextColor(COL_TEXT),
            ));
        });

        // ── Body — rebuilt by rebuild_lobby_ui_on_change ─────────────
        root.spawn((
            ConsoleListRoot,
            Node {
                flex_direction: FlexDirection::Column,
                flex_grow: 1.0,
                min_height: Val::Px(0.0),
                row_gap: Val::Px(6.0),
                ..default()
            },
        ));

        // ── Station detail panel (portrait: sibling; landscape: built inline) ──
        root.spawn((
            StationDetailPanel,
            Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.078, 0.090, 0.110, 0.6)),
        ))
        .with_children(|detail| {
            // Header row
            detail.spawn(Node { flex_direction: FlexDirection::Row, justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Center, ..default() })
            .with_children(|dh| {
                dh.spawn(Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(3.0), ..default() })
                .with_children(|titles| {
                    titles.spawn((
                        Text::new("// At your station"),
                        TextFont { font: mono.clone(), font_size: 8.0, ..default() },
                        TextColor(COL_TEXT_DIM),
                    ));
                    titles.spawn((
                        StationDetailTitle,
                        Text::new("— SELECT A STATION —"),
                        TextFont { font: chakra.clone(), font_size: 12.0, ..default() },
                        TextColor(COL_TEXT_DIM),
                    ));
                });
            });
            // Consoles label
            detail.spawn((
                Text::new("CONSOLES"),
                TextFont { font: chakra.clone(), font_size: 8.0, ..default() },
                TextColor(COL_TEXT_DIM),
            ));
            // Console chips container
            detail.spawn((
                StationDetailConsoles,
                Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(5.0),
                    row_gap: Val::Px(5.0),
                    ..default()
                },
            ));
            // Complexity row
            detail.spawn(Node {
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                padding: UiRect::top(Val::Px(6.0)),
                ..default()
            })
            .with_children(|cmplx| {
                cmplx.spawn((
                    Text::new("COMPLEXITY"),
                    TextFont { font: chakra.clone(), font_size: 9.0, ..default() },
                    TextColor(COL_TEXT_DIM),
                ));
                // Segmented control
                cmplx.spawn((
                    ComplexitySegControl,
                    Node { flex_direction: FlexDirection::Row, ..default() },
                    BackgroundColor(COL_GRAPHITE_DARK),
                ))
                .with_children(|seg| {
                    for (preset, label) in [("Low", "Low"), ("Std", "Normal")] {
                        seg.spawn((
                            ComplexityOptionButton(preset.to_string()),
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                                ..default()
                            },
                            BackgroundColor(COL_GRAPHITE_DARK),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                Text::new(label),
                                TextFont { font: chakra.clone(), font_size: 9.0, ..default() },
                                TextColor(COL_TEXT_DIM),
                            ));
                        });
                    }
                });
            });
        });

        // ── Footer ───────────────────────────────────────────────────
        root.spawn(Node {
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::SpaceBetween,
            align_items: AlignItems::Center,
            padding: UiRect::axes(Val::Px(2.0), Val::Px(0.0)),
            ..default()
        })
        .with_children(|foot| {
            foot.spawn((
                FooterStatus,
                Text::new(""),
                TextFont { font: mono.clone(), font_size: 8.0, ..default() },
                TextColor(COL_TEXT_DIM),
            ));
            foot.spawn((
                EngageButton,
                Button,
                Node {
                    padding: UiRect::axes(Val::Px(18.0), Val::Px(12.0)),
                    margin: UiRect { bottom: Val::Px(30.0), ..default() },
                    ..default()
                },
                BackgroundColor(COL_GRAPHITE_DARK),
                Visibility::Hidden,
            ))
            .with_children(|btn| {
                btn.spawn((
                    Text::new("ENGAGE"),
                    TextFont { font: chakra.clone(), font_size: 11.0, ..default() },
                    TextColor(COL_TEXT_DIM),
                ));
            });
        });
    });

    // ── Game-over overlay ──────────────────────────────────────────────
    commands.spawn((
        GameOverScreen,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(0.0),
            right: Val::Px(0.0),
            top: Val::Px(0.0),
            bottom: Val::Px(0.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(16.0),
            padding: UiRect::all(Val::Px(24.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.85)),
        Visibility::Hidden,
    ))
    .with_children(|overlay| {
        overlay.spawn((
            Text::new("GAME OVER"),
            TextFont { font: chakra.clone(), font_size: 28.0, ..default() },
            TextColor(COL_AMBER),
        ));
        overlay.spawn((
            GameOverReasonText,
            Text::new(""),
            TextFont { font: mono.clone(), font_size: 14.0, ..default() },
            TextColor(COL_TEXT),
        ));
    });
}

fn detect_initial_orientation(
    windows: Query<&Window>,
    mut landscape: ResMut<LandscapeMode>,
) {
    if let Ok(window) = windows.single() {
        landscape.0 = window.width() > window.height();
    }
}

fn detect_orientation_change(
    windows: Query<&Window, Changed<Window>>,
    mut landscape: ResMut<LandscapeMode>,
) {
    for window in windows.iter() {
        let is_landscape = window.width() > window.height();
        if landscape.0 != is_landscape {
            landscape.0 = is_landscape;
        }
    }
}

// ── Systems ────────────────────────────────────────────────────────

fn apply_inbound_messages(
    mut reader: MessageReader<InboundServerMessage>,
    mut lobby: ResMut<LobbyState>,
    mut sim: ResMut<ClientSimState>,
    mut complexity: ResMut<ComplexityStore>,
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
        // Sync ComplexityStore when server confirms a preset change.
        if let ServerMessage::ComplexityChanged { console, preset_name } = &ev.0 {
            if let Some(choice) = complexity.choices.get_mut(console) {
                let _ = choice.select(preset_name);
            }
        }
        lobby.apply(&ev.0);
        sim.apply(&ev.0);
        // ShipView is updated by ShipViewPlugin's own system.
    }
}

fn toggle_lobby_visibility_on_phase(
    state: Res<LobbyState>,
    mut roots: Query<&mut Visibility, With<LobbyRoot>>,
) {
    let in_lobby = state.phase == GamePhase::Lobby;
    for mut vis in roots.iter_mut() {
        *vis = if in_lobby { Visibility::Visible } else { Visibility::Hidden };
    }
}

fn toggle_game_over_visibility(
    state: Res<LobbyState>,
    mut screens: Query<&mut Visibility, With<GameOverScreen>>,
    mut reason_texts: Query<&mut Text, With<GameOverReasonText>>,
) {
    let is_game_over = state.phase == GamePhase::GameOver;
    for mut vis in screens.iter_mut() {
        *vis = if is_game_over { Visibility::Visible } else { Visibility::Hidden };
    }
    if is_game_over {
        if let Some(reason) = &state.game_over_reason {
            for mut text in reason_texts.iter_mut() {
                text.0 = reason.clone();
            }
        }
    }
}

fn rebuild_lobby_ui_on_change(
    mut commands: Commands,
    state: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    landscape: Res<LandscapeMode>,
    console_root: Query<Entity, With<ConsoleListRoot>>,
    children_q: Query<&Children>,
    asset_server: Res<AssetServer>,
) {
    if !state.is_changed() && !token.is_changed() && !landscape.is_changed() {
        return;
    }
    let chakra = asset_server.load("fonts/ChakraPetch-SemiBold.ttf");
    let mono   = asset_server.load("fonts/JetBrainsMono-Regular.ttf");

    let view = LobbyView::new(&state, &token.0);

    let Ok(body_root) = console_root.single() else { return };

    // Clear existing body children.
    if let Ok(children) = children_q.get(body_root) {
        for child in children.iter() {
            commands.entity(child).despawn();
        }
    }

    // Update ConsoleListRoot flex_direction based on orientation.
    commands.entity(body_root).insert(Node {
        flex_direction: if landscape.0 { FlexDirection::Row } else { FlexDirection::Column },
        flex_grow: 1.0,
        min_height: Val::Px(0.0),
        column_gap: Val::Px(8.0),
        row_gap: Val::Px(6.0),
        ..default()
    });

    // In both orientations, spawn the station list as the first column/section.
    let station_slots = view.station_slots();
    commands.entity(body_root).with_children(|body| {
        body.spawn(Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            overflow: Overflow::clip_y(),
            row_gap: Val::Px(6.0),
            ..default()
        })
        .with_children(|col| {
            for slot in &station_slots {
                spawn_station_row(col, slot, &chakra, &mono);
            }
        });

        // In landscape, also build the detail panel content inline as the right column.
        if landscape.0 {
            let my_mine_slot = station_slots.iter().find(|s| matches!(s, StationSlot::Mine { .. }));
            spawn_detail_column(body, my_mine_slot, &state, &token.0, &chakra, &mono);
        }
    });
}

fn refresh_engage_button(
    state: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    mut engage: Query<&mut Visibility, With<EngageButton>>,
) {
    if !state.is_changed() && !token.is_changed() {
        return;
    }
    let view = LobbyView::new(&state, &token.0);
    if let Ok(mut vis) = engage.single_mut() {
        *vis = if view.is_captain() && state.phase == GamePhase::Lobby && view.all_stations_filled() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Spawn the station list row for one slot using the new visual design.
fn spawn_station_row(
    parent: &mut ChildSpawnerCommands,
    slot: &StationSlot,
    chakra: &Handle<Font>,
    mono: &Handle<Font>,
) {
    match slot {
        StationSlot::Available { station, short_code, description, rank, consoles, .. } => {
            let mut row = parent.spawn((
                StationButton(station.clone()),
                Button,
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(10.0)),
                    column_gap: Val::Px(10.0),
                    ..default()
                },
                BackgroundColor(COL_GRAPHITE),
            ));
            row.with_children(|inner| {
                spawn_station_row_inner(inner, short_code, station, description, rank, consoles, false, chakra, mono);
            });
        }
        StationSlot::Occupied { station, short_code, description, rank, consoles, holder_name, .. } => {
            parent.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(10.0)),
                    column_gap: Val::Px(10.0),
                    ..default()
                },
                BackgroundColor(COL_GRAPHITE_DARK),
            ))
            .with_children(|inner| {
                spawn_station_row_inner(inner, short_code, station, description, rank, consoles, false, chakra, mono);
                // Holder badge on the right
                inner.spawn(Node { flex_grow: 1.0, ..default() });
                inner.spawn((
                    Text::new(holder_name.to_uppercase()),
                    TextFont { font: mono.clone(), font_size: 9.0, ..default() },
                    TextColor(COL_TEXT_DIM),
                ));
            });
        }
        StationSlot::Mine { station, short_code, description, rank, consoles, .. } => {
            parent.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(10.0)),
                    column_gap: Val::Px(10.0),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.373, 0.847, 0.910, 0.07)),
            ))
            .with_children(|inner| {
                spawn_station_row_inner(inner, short_code, station, description, rank, consoles, true, chakra, mono);
                // Spacer + Release button
                inner.spawn(Node { flex_grow: 1.0, ..default() });
                inner.spawn((
                    ReleaseStationButton,
                    Button,
                    Node {
                        padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                        ..default()
                    },
                    BackgroundColor(COL_EDGE),
                ))
                .with_children(|btn| {
                    btn.spawn((
                        Text::new("LEAVE"),
                        TextFont { font: chakra.clone(), font_size: 9.0, ..default() },
                        TextColor(COL_TEXT_DIM),
                    ));
                });
            });
        }
        StationSlot::Spectator { player_name } => {
            parent.spawn((
                Node {
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(COL_GRAPHITE_DARK),
            ))
            .with_children(|inner| {
                inner.spawn((
                    Text::new(format!("{} — SPECTATING", player_name.to_uppercase())),
                    TextFont { font: chakra.clone(), font_size: 9.0, ..default() },
                    TextColor(COL_TEXT_DIM),
                ));
            });
        }
    }
}

/// Inner content of a station row: short-code glyph, name/description column, consoles meta.
fn spawn_station_row_inner(
    parent: &mut ChildSpawnerCommands,
    short_code: &str,
    station: &str,
    description: &str,
    rank: &str,
    consoles: &[Console],
    is_mine: bool,
    chakra: &Handle<Font>,
    mono: &Handle<Font>,
) {
    let name_color = if is_mine { COL_SIGNAL } else { COL_TEXT };
    let meta_color = if is_mine { Color::srgba(0.373, 0.847, 0.910, 0.6) } else { COL_TEXT_DIM };

    // Short-code glyph
    parent.spawn((
        Text::new(short_code.to_uppercase()),
        TextFont { font: mono.clone(), font_size: 11.0, ..default() },
        TextColor(if is_mine { COL_SIGNAL } else { COL_EDGE }),
    ));

    // Name + description column
    parent.spawn(Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(2.0), flex_grow: 1.0, ..default() })
    .with_children(|col| {
        col.spawn((
            Text::new(station.to_uppercase()),
            TextFont { font: chakra.clone(), font_size: 11.0, ..default() },
            TextColor(name_color),
        ));
        if !description.is_empty() {
            col.spawn((
                Text::new(description),
                TextFont { font: mono.clone(), font_size: 9.0, ..default() },
                TextColor(meta_color),
            ));
        }
    });

    // Rank + consoles meta
    if !rank.is_empty() || !consoles.is_empty() {
        parent.spawn(Node { flex_direction: FlexDirection::Column, align_items: AlignItems::FlexEnd, row_gap: Val::Px(2.0), ..default() })
        .with_children(|meta| {
            if !rank.is_empty() {
                meta.spawn((
                    Text::new(rank.to_uppercase()),
                    TextFont { font: chakra.clone(), font_size: 8.0, ..default() },
                    TextColor(meta_color),
                ));
            }
            if !consoles.is_empty() {
                let console_names: Vec<&str> = consoles.iter().map(|c| c.display_name()).collect();
                meta.spawn((
                    Text::new(console_names.join(" · ")),
                    TextFont { font: mono.clone(), font_size: 8.0, ..default() },
                    TextColor(meta_color),
                ));
            }
        });
    }
}

/// Spawn the detail panel as an inline column (used in landscape mode).
fn spawn_detail_column(
    parent: &mut ChildSpawnerCommands,
    mine_slot: Option<&StationSlot>,
    state: &LobbyState,
    _my_token: &str,
    chakra: &Handle<Font>,
    mono: &Handle<Font>,
) {
    let (title, consoles, preset): (&str, Vec<Console>, Option<&str>) = match mine_slot {
        Some(StationSlot::Mine { station, consoles, .. }) => {
            let preset = consoles.first()
                .and_then(|c| state.complexity.get(c).map(|s| s.as_str()));
            (station.as_str(), consoles.clone(), preset)
        }
        _ => ("— SELECT A STATION —", vec![], None),
    };

    parent.spawn((
        Node {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            row_gap: Val::Px(8.0),
            padding: UiRect::axes(Val::Px(12.0), Val::Px(10.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.078, 0.090, 0.110, 0.6)),
    ))
    .with_children(|detail| {
        // Header
        detail.spawn(Node { flex_direction: FlexDirection::Column, row_gap: Val::Px(3.0), ..default() })
        .with_children(|titles| {
            titles.spawn((
                Text::new("// At your station"),
                TextFont { font: mono.clone(), font_size: 8.0, ..default() },
                TextColor(COL_TEXT_DIM),
            ));
            titles.spawn((
                Text::new(title.to_uppercase()),
                TextFont { font: chakra.clone(), font_size: 12.0, ..default() },
                TextColor(if mine_slot.is_some() { COL_TEXT } else { COL_TEXT_DIM }),
            ));
        });
        // Consoles label
        detail.spawn((
            Text::new("CONSOLES"),
            TextFont { font: chakra.clone(), font_size: 8.0, ..default() },
            TextColor(COL_TEXT_DIM),
        ));
        // Console chips
        detail.spawn(Node {
            flex_direction: FlexDirection::Row,
            flex_wrap: FlexWrap::Wrap,
            column_gap: Val::Px(5.0),
            row_gap: Val::Px(5.0),
            ..default()
        })
        .with_children(|chips| {
            for c in &consoles {
                chips.spawn((
                    Node {
                        padding: UiRect::axes(Val::Px(7.0), Val::Px(4.0)),
                        ..default()
                    },
                    BackgroundColor(COL_GRAPHITE),
                ))
                .with_children(|chip| {
                    chip.spawn((
                        Text::new(c.display_name().to_uppercase()),
                        TextFont { font: chakra.clone(), font_size: 9.0, ..default() },
                        TextColor(COL_TEXT),
                    ));
                });
            }
        });
        // Complexity row (only show when the player owns this station)
        if mine_slot.is_some() {
            detail.spawn(Node {
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                padding: UiRect::top(Val::Px(6.0)),
                ..default()
            })
            .with_children(|cmplx| {
                cmplx.spawn((
                    Text::new("COMPLEXITY"),
                    TextFont { font: chakra.clone(), font_size: 9.0, ..default() },
                    TextColor(COL_TEXT_DIM),
                ));
                cmplx.spawn((
                    Node { flex_direction: FlexDirection::Row, ..default() },
                    BackgroundColor(COL_GRAPHITE_DARK),
                ))
                .with_children(|seg| {
                    for (p, label) in [("Low", "Low"), ("Std", "Normal")] {
                        let is_active = preset == Some(p);
                        seg.spawn((
                            ComplexityOptionButton(p.to_string()),
                            Button,
                            Node {
                                padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                                ..default()
                            },
                            BackgroundColor(if is_active { COL_EDGE } else { COL_GRAPHITE_DARK }),
                        ))
                        .with_children(|btn| {
                            btn.spawn((
                                Text::new(label),
                                TextFont { font: chakra.clone(), font_size: 9.0, ..default() },
                                TextColor(if is_active { COL_TEXT } else { COL_TEXT_DIM }),
                            ));
                        });
                    }
                });
            });
        }
    });
}

/// Update the persistent portrait-mode `StationDetailPanel` whenever state changes.
fn refresh_station_detail(
    state: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    landscape: Res<LandscapeMode>,
    detail_panel: Query<(Entity, &Children), With<StationDetailPanel>>,
    mut detail_title: Query<&mut Text, With<StationDetailTitle>>,
    mut detail_consoles: Query<Entity, With<StationDetailConsoles>>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    if !state.is_changed() && !token.is_changed() && !landscape.is_changed() {
        return;
    }
    // In landscape mode the detail is built inline; hide the portrait panel.
    let Ok((panel_entity, _)) = detail_panel.single() else { return };
    if landscape.0 {
        commands.entity(panel_entity).insert(Visibility::Hidden);
        return;
    }
    commands.entity(panel_entity).insert(Visibility::Inherited);

    let view = LobbyView::new(&state, &token.0);
    let slots = view.station_slots();
    let mine_slot = slots.iter().find(|s| matches!(s, StationSlot::Mine { .. }));

    // Update title
    let title = match mine_slot {
        Some(StationSlot::Mine { station, .. }) => station.to_uppercase(),
        _ => "— SELECT A STATION —".to_string(),
    };
    if let Ok(mut text) = detail_title.single_mut() {
        **text = title;
    }

    // Rebuild console chips
    if let Ok(chips_root) = detail_consoles.single_mut() {
        commands.entity(chips_root).despawn_related::<Children>();
        if let Some(StationSlot::Mine { consoles, .. }) = mine_slot {
            let chakra: Handle<Font> = asset_server.load("fonts/ChakraPetch-SemiBold.ttf");
            commands.entity(chips_root).with_children(|chips| {
                for c in consoles {
                    chips.spawn((
                        Node {
                            padding: UiRect::axes(Val::Px(7.0), Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(COL_GRAPHITE),
                    ))
                    .with_children(|chip| {
                        chip.spawn((
                            Text::new(c.display_name().to_uppercase()),
                            TextFont { font: chakra.clone(), font_size: 9.0, ..default() },
                            TextColor(COL_TEXT),
                        ));
                    });
                }
            });
        }
    }
}

/// Marks the text node *inside* the `ReadyPill` container.
#[derive(Component)]
struct ReadyPillText;

/// Update the crew count and ready pill in the header.
fn refresh_crew_header(
    state: Res<LobbyState>,
    mut current_q: Query<&mut Text, (With<CrewCountCurrent>, Without<CrewCountMax>, Without<ReadyPillText>)>,
    mut max_q: Query<&mut Text, (With<CrewCountMax>, Without<CrewCountCurrent>, Without<ReadyPillText>)>,
    mut pill_text_q: Query<&mut Text, (With<ReadyPillText>, Without<CrewCountCurrent>, Without<CrewCountMax>)>,
    mut pill_bg_q: Query<&mut BackgroundColor, With<ReadyPill>>,
) {
    if !state.is_changed() {
        return;
    }
    let stationed = state.players.iter().filter(|p| !p.consoles.is_empty()).count();
    let total = state.players.len();

    if let Ok(mut text) = current_q.single_mut() {
        **text = stationed.to_string();
    }
    if let Ok(mut text) = max_q.single_mut() {
        **text = total.to_string();
    }

    let all_assigned = stationed == total && total > 0;
    if let Ok(mut bg) = pill_bg_q.single_mut() {
        bg.0 = if all_assigned {
            Color::srgba(0.373, 0.847, 0.910, 0.12)
        } else {
            Color::srgba(0.941, 0.627, 0.125, 0.08)
        };
    }
    if let Ok(mut text) = pill_text_q.single_mut() {
        **text = if all_assigned { "READY".to_string() } else { "AWAITING".to_string() };
    }
}

/// Update the scenario intro block when LobbyState changes.
fn update_scenario_intro(
    state: Res<LobbyState>,
    mut block_q: Query<&mut Visibility, With<ScenarioIntroBlock>>,
    mut title_q: Query<&mut Text, (With<ScenarioIntroTitle>, Without<ScenarioIntroBody>)>,
    mut body_q: Query<&mut Text, (With<ScenarioIntroBody>, Without<ScenarioIntroTitle>)>,
) {
    if !state.is_changed() {
        return;
    }
    let has_content = !state.scenario_title.is_empty() || !state.scenario_body.is_empty();
    if let Ok(mut vis) = block_q.single_mut() {
        *vis = if has_content { Visibility::Visible } else { Visibility::Hidden };
    }
    if let Ok(mut text) = title_q.single_mut() {
        text.0 = state.scenario_title.clone();
    }
    if let Ok(mut text) = body_q.single_mut() {
        text.0 = state.scenario_body.clone();
    }
}

/// Update the footer status text.
fn refresh_footer_status(
    state: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    mut status_q: Query<&mut Text, With<FooterStatus>>,
) {
    if !state.is_changed() && !token.is_changed() {
        return;
    }
    let Ok(mut text) = status_q.single_mut() else { return };
    let view = LobbyView::new(&state, &token.0);
    **text = if view.is_captain() {
        if view.all_stations_filled() {
            "All stations filled — engage when ready".to_string()
        } else {
            "Waiting for crew to take their stations…".to_string()
        }
    } else if view.is_spectator() {
        "Select a station to join the crew".to_string()
    } else {
        String::new()
    };
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
        let slot = StationSlot::Available { station: s.clone(), short_code: String::new(), description: String::new(), rank: String::new(), consoles: vec![], preset_names: vec![] };
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

fn handle_release_station_button_press(
    mut interactions: Query<
        &Interaction,
        (Changed<Interaction>, With<Button>, With<ReleaseStationButton>),
    >,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter_mut() {
        if *interaction == Interaction::Pressed {
            outbound.write(OutboundClientMessage(
                crate::client_lobby::release_station_message(),
            ));
        }
    }
}

fn handle_complexity_option_press(
    mut interactions: Query<
        (&Interaction, &ComplexityOptionButton),
        (Changed<Interaction>, With<Button>),
    >,
    state: Res<LobbyState>,
    token: Res<LocalPlayerToken>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, ComplexityOptionButton(preset)) in interactions.iter_mut() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Find all consoles in the local player's current station and send
        // SetComplexity for each one.
        let view = LobbyView::new(&state, &token.0);
        for console in view.my_consoles() {
            outbound.write(OutboundClientMessage(ClientMessage::SetComplexity {
                console: console.clone(),
                preset_name: preset.clone(),
            }));
        }
    }
}

fn setup_helm_ui(_commands: Commands) {
    // Helm UI is now owned by HelmPanelPlugin (src/helm_panel.rs).
    // This startup system is retained as a no-op so the add_systems call
    // does not need to be changed.
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
            // Suppress if all teams are busy.
            let all_busy = sim.repair_teams.iter().all(|t| !matches!(t, crate::messages::TeamSlot::Idle));
            if all_busy {
                continue;
            }
            outbound.write(OutboundClientMessage(crate::client_sim::repair_message()));
        }
    }
}

fn refresh_repair_button(
    sim: Res<ClientSimState>,
    mut button: Query<&mut BackgroundColor, (With<RepairButton>, Without<RepairButtonLabel>)>,
    mut label: Query<(&mut Text, &mut TextColor), With<RepairButtonLabel>>,
) {
    if !sim.is_changed() {
        return;
    }
    let any_active = sim.repair_teams.iter().any(|t| !matches!(t, crate::messages::TeamSlot::Idle));
    for mut bg in button.iter_mut() {
        *bg = if any_active {
            BackgroundColor(Color::srgb(0.05, 0.30, 0.05))
        } else {
            BackgroundColor(Color::srgb(0.13, 0.27, 0.13))
        };
    }
    for (mut text, mut color) in label.iter_mut() {
        if any_active {
            **text = "TEAMS DISPATCHED".to_string();
            *color = TextColor(Color::srgb(0.5, 1.0, 0.5));
        } else {
            **text = "REPAIR".to_string();
            *color = TextColor(Color::srgb(0.5, 1.0, 0.5));
        }
    }
}

// ── Complexity dropdown / pop-up ───────────────────────────────────

/// Refresh complexity pop-up and dropdown visibility based on the store.
fn refresh_complexity_ui(
    store: Res<ComplexityStore>,
    mut popup: Query<&mut Visibility, (With<ComplexityPopupRoot>, Without<ComplexityDropdownRoot>)>,
    mut dropdown: Query<&mut Visibility, (With<ComplexityDropdownRoot>, Without<ComplexityPopupRoot>)>,
) {
    let choice = store.choices.get(&Console::Tactical);
    let Some(choice) = choice else { return };

    for mut vis in popup.iter_mut() {
        *vis = if choice.show_popup() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
    for mut vis in dropdown.iter_mut() {
        *vis = if choice.show_dropdown() && !choice.show_popup() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
    }
}

/// Handle presses on complexity preset buttons (both pop-up and dropdown).
fn handle_complexity_preset_press(
    interactions: Query<(&Interaction, &ComplexityPresetButton), Changed<Interaction>>,
    mut store: ResMut<ComplexityStore>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for (interaction, btn) in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Update local store selection.
        if let Some(choice) = store.choices.get_mut(&Console::Tactical) {
            let _ = choice.select(&btn.0);
        }
        // Send SetComplexity immediately so the server knows.
        outbound.write(OutboundClientMessage(
            client_complexity::set_complexity_message(Console::Tactical, &btn.0),
        ));
    }
}

/// Handle the confirm button on the complexity pop-up.
///
/// The preset was already selected (and `SetComplexity` sent) by
/// `handle_complexity_preset_press` when the user tapped a pop-up
/// preset button. Confirm merely closes the pop-up (the store was
/// already updated by `select()`, which sets `popup_shown = true`,
/// causing `refresh_complexity_ui` to hide it).
fn handle_complexity_popup_confirm(
    interactions: Query<&Interaction, (Changed<Interaction>, With<ComplexityPopupConfirm>)>,
    mut store: ResMut<ComplexityStore>,
    mut outbound: MessageWriter<OutboundClientMessage>,
) {
    for interaction in interactions.iter() {
        if *interaction != Interaction::Pressed {
            continue;
        }
        // Ensure a preset is selected (default to Low if none was tapped).
        let need_send = {
            let choice = store.choices.get(&Console::Tactical);
            choice.map(|c| c.chosen.is_none()).unwrap_or(true)
        };
        if need_send {
            let _ = store.for_console(&Console::Tactical).select("Low");
            outbound.write(OutboundClientMessage(
                client_complexity::set_complexity_message(Console::Tactical, "Low"),
            ));
        }
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
    mut active: ResMut<ActiveConsole>,
    tab_root_q: Query<Entity, With<TabBarRoot>>,
    mut tab_vis_q: Query<&mut Visibility, With<TabBarRoot>>,
    children_q: Query<&Children>,
) {
    let view = LobbyView::new(&lobby, &token.0);
    let my_consoles = view.my_consoles();

    // If the player dropped the console they had selected, reset to auto-mode
    // so the remaining panel(s) are shown correctly.
    if let Some(c) = active.0.clone() {
        if !my_consoles.contains(&c) {
            active.0 = None;
        }
    }

    if !lobby.is_changed() && !token.is_changed() && !active.is_changed() {
        return;
    }

    let Ok(root) = tab_root_q.single() else { return };
    let in_game = lobby.phase == GamePhase::InProgress;
    let show_tabs = in_game && my_consoles.len() >= 2;
    let use_initials = my_consoles.len() >= 5;

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
            let padding = if use_initials {
                UiRect::axes(Val::Px(6.0), Val::Px(6.0))
            } else {
                UiRect::axes(Val::Px(14.0), Val::Px(6.0))
            };
            let mut btn = parent.spawn((
                Node {
                    padding,
                    ..default()
                },
                BackgroundColor(bg),
            ));
            btn.insert((Button, TabButton(console.clone())));
            btn.with_children(|inner| {
                inner.spawn((
                    Text::new(if use_initials { console.initial() } else { console.display_name() }),
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

// ── Hideable element registration ────────────────────────────────

/// One-shot system: scans all existing `HideableElement` markers and
/// registers their names in the `HideableElementRegistry`.
fn register_hideable_elements(
    mut registry: ResMut<HideableElementRegistry>,
    elements: Query<&HideableElement>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    for element in elements.iter() {
        registry.register(element.0.clone());
    }
    *done = true;
}

/// Reads the `ComplexityStore` and applies hide/show to `HideableElement`
/// entities when the effective preset changes for the local player's consoles.
///
/// - Only affects consoles held by the local player
/// - Unknown TOML element names produce runtime warnings
/// - Hidden elements get `Display::None`; restored elements get `Display::Flex`
fn sync_complexity_hiding(
    mut registry: ResMut<HideableElementRegistry>,
    store: Res<ComplexityStore>,
    mut elements: Query<(&mut Node, &HideableElement)>,
    token: Res<LocalPlayerToken>,
    lobby: Res<LobbyState>,
) {
    // Guard: if neither resource changed, skip.
    if !store.is_changed() && !lobby.is_changed() && !token.is_changed() {
        return;
    }

    let view = LobbyView::new(&lobby, &token.0);
    for console in view.my_consoles() {
        let Some(choice) = store.choices.get(console) else {
            continue;
        };
        let current = choice.effective_preset().to_string();
        let last = registry.last_applied.get(console).cloned();

        if last.as_ref() == Some(&current) {
            continue;
        }

        let changes = registry.planned_changes(console, &current);

        // Log warnings for unknown element names from TOML.
        for name in &changes.unknown {
            bevy::log::warn!(
                "Hideable element '{name}' is in TOML hidden_elements for {console:?} \
                 but no UI element registered that name; check spelling or add a \
                 HideableElement(\"{name}\") marker"
            );
        }

        // Apply display: none / display: flex to matching entities.
        for (mut node, element) in elements.iter_mut() {
            if changes.to_hide.contains(&element.0) {
                node.display = bevy::ui::Display::None;
            } else if changes.to_show.contains(&element.0) {
                node.display = bevy::ui::Display::Flex;
            }
        }

        registry.apply_changes(&changes);
        registry.last_applied.insert(console.clone(), current);
    }
}

// ── Thin composition ────────────────────────────────────────────────────────

/// Register all client-side plugins onto `app`.
///
/// Call this from the WASM entry point (`wasm_client_init`) instead of
/// listing plugins individually.  Every panel plugin is registered here so
/// that `client/bridge.rs` remains a thin JS/WASM boundary with no
/// knowledge of the panel set.
///
/// Panel inventory:
/// - `ShipViewPlugin`          — ship-level broadcast resource
/// - `ClientAppPlugin`         — lobby UI + tab bar + complexity UI
/// - `PhoneBorderPlugin`       — diegetic phone bezel frame
/// - `CaptainPanelPlugin`      — view selector + red-alert toggle
/// - `HelmPanelPlugin`         — joystick + helm radar
/// - `WeaponsPanelPlugin`      — phaser / torpedo / weapons radar
/// - `RepairPanelPlugin`       — shape-matching repair console
/// - `PowerPanelPlugin`        — 6+2 power allocation console
/// - `SensorsPanelPlugin`      — long-range radar + science target designation
/// - `ShieldsPanelPlugin`      — 4-quadrant HP bars + focus mechanic
/// - `NavigationPanelPlugin`   — system chart + impulse status + cancel
/// - `CommsPanelPlugin`        — comms console (placeholder)
pub fn add_client_plugins(app: &mut App) {
    app.add_plugins(ClientAppPlugin)
        .add_plugins(crate::gui::GuiPlugin)
        .add_plugins(crate::ship_view::ShipViewPlugin)
        .add_plugins(crate::phone_border::PhoneBorderPlugin)
        .add_plugins(crate::captain_panel::CaptainPanelPlugin)
        .add_plugins(crate::helm_panel::HelmPanelPlugin)
        .add_plugins(crate::weapons_panel::WeaponsPanelPlugin)
        .add_plugins(crate::repair_panel::RepairPanelPlugin)
        .add_plugins(crate::power_panel::PowerPanelPlugin)
        .add_plugins(crate::sensors_panel::SensorsPanelPlugin)
        .add_plugins(crate::shields_panel::ShieldsPanelPlugin)
        .add_plugins(crate::navigation_panel::NavigationPanelPlugin)
        .add_plugins(crate::comms_panel::CommsPanelPlugin);
}
