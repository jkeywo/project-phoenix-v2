//! Client-side Bevy app — in-game console UI and message routing.
//!
//! This plugin owns the `LobbyState` and `LocalPlayerToken` resources and
//! emits outbound `ClientMessage` events when in-game UI elements (the repair
//! button) are pressed.  Outbound emission is the only side effect that
//! escapes the plugin; the bridge layer (`client_bridge`) is responsible for
//! marshalling those events to/from JS.
//!
//! The complexity preset selector, first-use pop-up, and per-preset element
//! hiding moved to the HTML/JS shell in issue #461 (gui/complexity-ui.js,
//! gui/hideable-elements.js, gui/console-core.js).
//!
//! Inbound `ServerMessage` draining moved to pure JS in #460 — the gui/
//! state modules (lobby-state.js, sim-state.js, comms-state.js,
//! complexity-store.js) apply every inbound message in client.html. The
//! Bevy resources here no longer receive message-driven updates.
//!
//! Pre-#442 this module also rendered the Bevy lobby UI (`LobbyRoot`,
//! game-over overlay, station-detail panel, complexity segmented
//! control, engage button) and detected window orientation into a
//! `LandscapeMode` resource. All of that moved to the HTML/JS shell in
//! `client.html` (issues #439 / #440 / #441). The Bevy app now spawns
//! only a UI camera; HTML owns every chrome surface (bezel, lobby,
//! tab bar, game-over).
//!
//! The plugin is platform-agnostic; the bridge layer wires it together
//! with `DefaultPlugins` and the wasm-bindgen entry points.

use bevy::prelude::*;

use crate::client_lobby::{ActiveConsole, LobbyState, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::client_comms::ClientCommsState;
use crate::client_complexity::ComplexityStore;
use crate::client_elements::{handle_help_button_press, handle_help_overlay_dismiss};
use crate::messages::{ClientMessage, ServerMessage};
use crate::gui::{ConsoleRadar, GenericRadarWidget, RadarFilter};

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

// ── Marker components (in-game only) ───────────────────────────────

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

// Complexity pop-up / dropdown and hideable-element markers were removed in
// issue #461; the preset selector, first-use pop-up, and element hiding now
// live entirely in the HTML/JS shell (gui/complexity-ui.js,
// gui/hideable-elements.js, gui/console-core.js).

// ── System sets ────────────────────────────────────────────────────

/// Ordering labels for client-side `Update` systems.
///
/// `MessageProcessing` must run before `ConsoleUpdate` so that
/// `forward_inbound_messages` (in the bridge) has committed its results
/// before any system that reads the decoded messages runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, SystemSet)]
pub enum ClientSet {
    /// Drain inbound server messages and update lobby/sim state resources.
    MessageProcessing,
    /// Show/hide console panels based on the current lobby state.
    ConsoleUpdate,
}

// ── Plugin ─────────────────────────────────────────────────────────

pub struct ClientAppPlugin;

impl Plugin for ClientAppPlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
                Update,
                ClientSet::MessageProcessing.before(ClientSet::ConsoleUpdate),
            )
            .insert_resource(ClearColor(Color::srgb(
            10.0 / 255.0,
            10.0 / 255.0,
            26.0 / 255.0,
        )))
        .init_resource::<LobbyState>()
        .init_resource::<ClientSimState>()
        .init_resource::<ClientCommsState>()
        .init_resource::<LocalPlayerToken>()
        .init_resource::<ActiveConsole>()
        .init_resource::<ComplexityStore>()
        .add_message::<InboundServerMessage>()
        .add_message::<OutboundClientMessage>()
        .add_systems(Startup, (setup_ui_camera, setup_helm_ui))
        .add_systems(
            Update,
            (
                (handle_repair_button_press, refresh_repair_button),
                (handle_help_button_press, handle_help_overlay_dismiss),
                sync_radar_widgets_from_lobby,
            ),
        );
    }
}

// ── Setup ──────────────────────────────────────────────────────────

/// Spawn the 2D UI camera. `IsDefaultUiCamera` marks it as the target for
/// UI roots that don't carry an explicit `UiTargetCamera`, which Bevy 0.18
/// requires for text glyph extraction to resolve a camera deterministically.
///
/// Pre-#442 this lived inside `setup_lobby_ui`; that system was removed
/// when the Bevy lobby UI moved to HTML, so the camera spawn was lifted
/// into its own startup system to keep all remaining UI roots renderable.
fn setup_ui_camera(mut commands: Commands) {
    commands.spawn((Camera2d, IsDefaultUiCamera));
}

fn setup_helm_ui(_commands: Commands) {
    // Helm UI is now owned by HelmPanelPlugin (src/console/helm/client.rs).
    // This startup system is retained as a no-op so the add_systems call
    // does not need to be changed.
}

// ── Unified radar widget sync ────────────────────────────────────────

/// Per-frame sync of every console's `GenericRadarWidget` from
/// `LobbyState.ship_config`. Picks the right `*_shows` tag list (and helm
/// range) based on the widget's `ConsoleRadar` variant.
///
/// Replaces the previous per-console sync systems
/// (`sync_helm_radar_range`, `sync_sensors_radar_filter`, etc.) so all
/// TOML-driven radar configuration flows through one place. Server
/// viewscreen variants are skipped here; the server keeps its own
/// viewscreen-radar bridge in `src/server/radar.rs`.
fn sync_radar_widgets_from_lobby(
    lobby: Res<LobbyState>,
    mut widgets: Query<(&ConsoleRadar, &mut GenericRadarWidget)>,
) {
    let cfg = &lobby.ship_config;
    for (console, mut widget) in widgets.iter_mut() {
        let shows: &[String] = match console {
            ConsoleRadar::Helm => &cfg.helm_radar_shows,
            ConsoleRadar::Sensors => &cfg.sensors_radar_shows,
            ConsoleRadar::Navigation => &cfg.nav_chart_shows,
            ConsoleRadar::Tactical => &cfg.tactical_radar_shows,
            // Viewscreen radars on the server are configured directly by
            // `src/server/radar.rs::spawn_viewscreen_radar_widgets`.
            ConsoleRadar::ViewscreenHelm
            | ConsoleRadar::ViewscreenScience
            | ConsoleRadar::ViewscreenSystemChart
            | ConsoleRadar::ViewscreenNav => continue,
        };
        if !shows.is_empty() {
            widget.filter = RadarFilter::from_shows(shows);
        }
        if matches!(console, ConsoleRadar::Helm) {
            widget.range = cfg.helm_radar_range;
        }
        if matches!(console, ConsoleRadar::Sensors) {
            widget.range = cfg.sensors_radar_range;
        }
    }
}

// ── Message draining ───────────────────────────────────────────────
//
// `apply_inbound_messages` was deleted in #460: the JS state modules in
// gui/ (lobby-state.js, sim-state.js, comms-state.js, complexity-store.js)
// now apply every inbound ServerMessage in client.html's handleMessage(),
// including the StationAssigned active-console reconciliation and the
// ComplexityChanged store sync this drain used to perform. The
// `LobbyState` / `ClientSimState` / `ClientCommsState` resources remain
// registered for the Bevy systems below but no longer receive updates;
// those systems are removed in later slices (#461/#462).

// ── Repair button ──────────────────────────────────────────────────

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
            let all_busy = sim
                .repair_teams
                .iter()
                .all(|t| !matches!(t, crate::messages::TeamSlot::Idle));
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
    let any_active = sim
        .repair_teams
        .iter()
        .any(|t| !matches!(t, crate::messages::TeamSlot::Idle));
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

// Complexity pop-up / dropdown systems (refresh_complexity_ui,
// handle_complexity_preset_press, handle_complexity_popup_confirm) and the
// hideable-element systems (register_hideable_elements, sync_complexity_hiding)
// were removed in issue #461. The preset selector, first-use pop-up, and
// per-preset element hiding now live in the HTML/JS shell:
//   - gui/complexity-ui.js     — pop-up plan / segmented selector / SetComplexity
//   - gui/hideable-elements.js — preset → hidden_elements table + DOM toggling
//   - gui/console-core.js      — applies hiding on each __updateConsole push

// ── Thin composition ────────────────────────────────────────────────────────

/// Register all client-side plugins onto `app`.
///
/// Call this from the WASM entry point (`wasm_client_init`) instead of
/// listing plugins individually.  Every panel plugin is registered here so
/// that `client/bridge.rs` remains a thin JS/WASM boundary with no
/// knowledge of the panel set.
///
/// Panel inventory (all per-console Bevy panels deleted per #456–#458):
/// - `ShipViewPlugin`   — ship-level broadcast resource
/// - `ClientAppPlugin`  — message routing + radar widget sync
/// - `PhoneBorderPlugin`— loads `PhoneAssets` and drives `DeviceOrientation`
pub fn add_client_plugins(app: &mut App) {
    app.add_plugins(ClientAppPlugin)
        .add_plugins(crate::gui::GuiPlugin)
        .add_plugins(crate::ship_view::ShipViewPlugin)
        .add_plugins(crate::phone_border::PhoneBorderPlugin);
}
