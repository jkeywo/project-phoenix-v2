//! Client-side Bevy app — in-game console UI and message routing.
//!
//! This plugin owns the `LobbyState` and `LocalPlayerToken` resources and
//! emits outbound `ClientMessage` events when in-game UI elements (repair
//! button, complexity popup) are pressed.  Outbound emission is the only
//! side effect that escapes the plugin; the bridge layer (`client_bridge`)
//! is responsible for marshalling those events to/from JS.
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

use crate::client_lobby::{ActiveConsole, LobbyState, LobbyView, LocalPlayerToken};
use crate::client_sim::ClientSimState;
use crate::client_comms::ClientCommsState;
use crate::client_complexity::{self, ComplexityStore};
use crate::client_elements::{
    handle_help_button_press, handle_help_overlay_dismiss, HideableElementRegistry,
};
use crate::messages::{ClientMessage, Console, ServerMessage};
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

/// Marks a UI element that can be hidden by complexity preset `hidden_elements`.
/// The string name must match an entry in the complexity TOML for this console.
#[derive(Component)]
pub struct HideableElement(pub String);

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
        .init_resource::<HideableElementRegistry>()
        .add_message::<InboundServerMessage>()
        .add_message::<OutboundClientMessage>()
        .add_systems(Startup, (setup_ui_camera, setup_helm_ui))
        .add_systems(
            Update,
            (
                (handle_repair_button_press, refresh_repair_button),
                (
                    refresh_complexity_ui,
                    handle_complexity_preset_press,
                    handle_complexity_popup_confirm,
                ),
                (register_hideable_elements, sync_complexity_hiding),
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
/// Panel inventory (all per-console Bevy panels deleted per #456–#458):
/// - `ShipViewPlugin`   — ship-level broadcast resource
/// - `ClientAppPlugin`  — message routing + complexity UI + radar widget sync
/// - `PhoneBorderPlugin`— loads `PhoneAssets` and drives `DeviceOrientation`
pub fn add_client_plugins(app: &mut App) {
    app.add_plugins(ClientAppPlugin)
        .add_plugins(crate::gui::GuiPlugin)
        .add_plugins(crate::ship_view::ShipViewPlugin)
        .add_plugins(crate::phone_border::PhoneBorderPlugin);
}
