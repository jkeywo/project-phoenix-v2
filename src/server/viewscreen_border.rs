//! Viewscreen border frame.
//!
//! This module owns the viewscreen border frame around the 3D scene.
//! It loads the ten normal-state and ten alert-variant border PNGs,
//! the HUD font, and the Red Alert vignette WGSL shader through
//! `AssetServer` at startup, and spawns the frame immediately at
//! startup (visible in both Lobby and InProgress phases).
//!
//! Server-only — gated by the `server` feature in `lib.rs`.
//!
//! ## Layout
//!
//! Children of a viewport-filling root `Node`, in spawn order (which is
//! Bevy UI's back-to-front order):
//!
//! 1. **Vignette `MaterialNode<RedAlertVignetteMaterial>`** — full-bleed,
//!    spawned first so the border sprites occlude its outermost ring.
//! 2. **4 corners** (240×140 px) anchored to each viewport corner.
//! 3. **Top cap** (320×56 px) centred along the top edge.
//! 4. **Bottom cap** (520×56 px) centred along the bottom edge.
//! 5. **4 edges** using `NodeImageMode::Tiled` to fill the gap between
//!    corners and caps along each side.
//!
//! Bevy UI's default render order layers the frame above the 3D scene
//! cameras. The existing `ViewDirectionLabel` (top-centre) and
//! `FpsText` (top-right) sit at fixed pixel positions outside the
//! corner/cap footprint and remain visible.
//!
//! ## Bottom cap HUD strip
//!
//! Two separate node trees are swapped on phase transition:
//!
//! - **Lobby strip** (spawned at startup): CLOCK / PLAYERS / STATUS.
//!   CLOCK shows the real-world `hh:mm:ss` via `js_sys::Date`.
//!   PLAYERS shows `connected / max_players` from `SessionManager` and
//!   `ShipStations`. STATUS shows `AWAITING CREW` when any connected
//!   player has no console, `READY FOR DEPARTURE` when all do.
//! - **In-game strip** (spawned on `InProgress`, lobby strip despawned):
//!   HEADING / HULL / CONDITION driven by `ShipState`.
//!
//! ## Red Alert
//!
//! When `ShipState.red_alert` flips:
//!
//! - Each border `ImageNode`'s texture handle is swapped instantly
//!   between its normal and alert variant by [`swap_border_textures`].
//! - The vignette material's `intensity` uniform is driven each frame
//!   by [`drive_vignette_intensity`], which calls the pure helper
//!   [`pulse_intensity`] to combine a quarter-second on/off ease with a
//!   1.3-second sine pulse between [`MIN_INTENSITY`] and
//!   [`MAX_INTENSITY`].
//!
//! The Red Alert visual is owned end-to-end here. The previous CSS
//! vignette in `server.html` was removed in the same change.

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::ui::widget::NodeImageMode;
use bevy::ui_render::prelude::{MaterialNode, UiMaterial, UiMaterialPlugin};
#[cfg(target_arch = "wasm32")]
use js_sys::Date;

use crate::console_ai_plugin::ConsoleComplexityState;
use crate::lobby::Sessions;
use crate::messages::GamePhase;
use crate::ship_state::ShipState;
use crate::simulation::ShipHullIntegrity;
use crate::stations_config::{ShipStations, StationDef};

// ── Layout constants ─────────────────────────────────────────────────
//
// All dimensions are in CSS pixels and match the source PNG sizes
// exactly. No proportional scaling — the server is a desktop / large-
// screen target.

pub(crate) const CORNER_W: f32 = 240.0;
pub(crate) const CORNER_H: f32 = 140.0;
const CAP_TOP_W: f32 = 320.0;
const CAP_BOTTOM_W: f32 = 520.0;
const CAP_H: f32 = 56.0;
const EDGE_THICKNESS: f32 = 44.0;

// ── Pulse constants ──────────────────────────────────────────────────
//
// Visual tuning, not gameplay. All module-private.

/// Quarter-second ease window when toggling Red Alert on or off.
const EASE_DURATION: f32 = 0.25;

/// Sine pulse period when Red Alert is steady-on.
const PULSE_PERIOD: f32 = 1.3;

/// Vignette intensity at the trough of the sine pulse.
const MIN_INTENSITY: f32 = 0.55;

/// Vignette intensity at the crest of the sine pulse.
const MAX_INTENSITY: f32 = 1.0;

// ── HUD constants ────────────────────────────────────────────────────

/// Signal-cyan `#5fd8e8` — designation + status values when nominal.
const COLOR_SIGNAL_CYAN: Color = Color::srgb(0.373, 0.847, 0.910);

/// Alert-red `#ff3344` — designation + status values at red alert.
const COLOR_ALERT_RED: Color = Color::srgb(1.0, 0.2, 0.267);

/// Neutral `#b8c0c8` — status labels (never swap colour).
const COLOR_NEUTRAL_LABEL: Color = Color::srgb(0.722, 0.753, 0.784);

// ── Lobby screen constants ────────────────────────────────────────────

/// Dim text used for labels and placeholders on the lobby screen.
const COLOR_TEXT_DIM: Color = Color::srgb(0.416, 0.447, 0.482);
/// Dark graphite background for cards and counter boxes.
const COLOR_GRAPHITE_DARK: Color = Color::srgb(0.078, 0.090, 0.110);
/// Mid graphite for inset elements (glyph boxes, console pills).
const COLOR_GRAPHITE: Color = Color::srgb(0.133, 0.149, 0.176);
/// Edge / border colour for inactive UI chrome.
const COLOR_EDGE: Color = Color::srgb(0.227, 0.251, 0.286);
/// Amber used for the "awaiting" ready badge value.
const COLOR_AMBER: Color = Color::srgb(0.941, 0.627, 0.125);
/// Signal-deep `#2a8a96` — used for `is_low` border on complexity pill.
const COLOR_SIGNAL_DEEP: Color = Color::srgb(0.165, 0.541, 0.588);

const LOBBY_KICKER_SIZE: f32 = 10.0;
const LOBBY_TITLE_SIZE: f32 = 20.0;
const LOBBY_LABEL_SIZE: f32 = 8.0;
const LOBBY_VALUE_SIZE: f32 = 13.0;
const LOBBY_CARD_NAME_SIZE: f32 = 12.0;
const LOBBY_CARD_SMALL_SIZE: f32 = 8.0;

/// Static designation displayed on the top cap.
const DESIGNATION_TEXT: &str = "AEV-074 \u{00B7} PHOENIX";

const DESIGNATION_FONT_SIZE: f32 = 18.0;
const STATUS_LABEL_FONT_SIZE: f32 = 11.0;
const STATUS_VALUE_FONT_SIZE: f32 = 18.0;

// ── Resources ────────────────────────────────────────────────────────

/// Holds asset handles for the viewscreen border frame.
///
/// Inserted at startup by [`ViewscreenBorderPlugin`]. Holding the handles
/// in a resource keeps the assets alive (Bevy reference-counts handles)
/// and gives later systems a stable place to look them up.
#[derive(Resource, Debug, Clone)]
pub struct ViewscreenAssets {
    pub corner_tl: Handle<Image>,
    pub corner_tr: Handle<Image>,
    pub corner_bl: Handle<Image>,
    pub corner_br: Handle<Image>,
    pub edge_top: Handle<Image>,
    pub edge_bottom: Handle<Image>,
    pub edge_left: Handle<Image>,
    pub edge_right: Handle<Image>,
    pub cap_top: Handle<Image>,
    pub cap_bottom: Handle<Image>,
    // Alert variants — swapped in by [`swap_border_textures`] on red_alert change.
    pub corner_tl_alert: Handle<Image>,
    pub corner_tr_alert: Handle<Image>,
    pub corner_bl_alert: Handle<Image>,
    pub corner_br_alert: Handle<Image>,
    pub edge_top_alert: Handle<Image>,
    pub edge_bottom_alert: Handle<Image>,
    pub edge_left_alert: Handle<Image>,
    pub edge_right_alert: Handle<Image>,
    pub cap_top_alert: Handle<Image>,
    pub cap_bottom_alert: Handle<Image>,
    /// Display font for HUD readouts (added in #184).
    pub font_display: Handle<Font>,
    /// Monospace font for the HUD numeric value cells (added in #184).
    pub font_mono: Handle<Font>,
}

/// Cached handle to the single `RedAlertVignetteMaterial` instance,
/// so `drive_vignette_intensity` can mutate its uniform without a query.
#[derive(Resource, Debug, Clone)]
struct VignetteMaterialHandle(Handle<RedAlertVignetteMaterial>);

// ── Marker components ────────────────────────────────────────────────

/// Marker for the root `Node` that owns the entire border frame.
///
/// Despawning this entity (with descendants) tears down all border
/// `ImageNode` children and the vignette material node in one shot.
#[derive(Component)]
struct ViewscreenBorderRoot;

/// Identifies which border slot an `ImageNode` occupies, so
/// [`swap_border_textures`] can rewrite each handle on `red_alert`
/// change without coupling spawn order to lookup order.
#[derive(Component, Copy, Clone, Debug, PartialEq, Eq)]
enum BorderSlot {
    CornerTl,
    CornerTr,
    CornerBl,
    CornerBr,
    EdgeTop,
    EdgeBottom,
    EdgeLeft,
    EdgeRight,
    CapTop,
    CapBottom,
}

impl BorderSlot {
    fn handle<'a>(self, assets: &'a ViewscreenAssets, alert: bool) -> &'a Handle<Image> {
        match (self, alert) {
            (Self::CornerTl, false) => &assets.corner_tl,
            (Self::CornerTl, true) => &assets.corner_tl_alert,
            (Self::CornerTr, false) => &assets.corner_tr,
            (Self::CornerTr, true) => &assets.corner_tr_alert,
            (Self::CornerBl, false) => &assets.corner_bl,
            (Self::CornerBl, true) => &assets.corner_bl_alert,
            (Self::CornerBr, false) => &assets.corner_br,
            (Self::CornerBr, true) => &assets.corner_br_alert,
            (Self::EdgeTop, false) => &assets.edge_top,
            (Self::EdgeTop, true) => &assets.edge_top_alert,
            (Self::EdgeBottom, false) => &assets.edge_bottom,
            (Self::EdgeBottom, true) => &assets.edge_bottom_alert,
            (Self::EdgeLeft, false) => &assets.edge_left,
            (Self::EdgeLeft, true) => &assets.edge_left_alert,
            (Self::EdgeRight, false) => &assets.edge_right,
            (Self::EdgeRight, true) => &assets.edge_right_alert,
            (Self::CapTop, false) => &assets.cap_top,
            (Self::CapTop, true) => &assets.cap_top_alert,
            (Self::CapBottom, false) => &assets.cap_bottom,
            (Self::CapBottom, true) => &assets.cap_bottom_alert,
        }
    }
}

// ── Lobby screen marker components ───────────────────────────────────

/// Root panel of the server lobby screen overlay (fills the inner viewscreen
/// area). Toggled between Visible and Hidden on phase transitions.
#[derive(Component)]
struct LobbyScreenRoot;

/// Container for the 3×2 station card grid. Children are despawned and
/// rebuilt whenever session state changes.
#[derive(Component)]
struct LobbyGridRoot;

/// Marker on each station card (active or placeholder) inside the grid.
/// All entities with this component are despawned on each grid rebuild.
#[derive(Component)]
struct LobbyStationCard;

/// Text node showing the current crew count ("2/3").
#[derive(Component)]
struct LobbyCrewDisplay;

/// Text node showing the ready status ("READY TO LAUNCH" / "AWAITING SELECTION").
#[derive(Component)]
struct LobbyReadyVal;

// ── HUD marker components ────────────────────────────────────────────

/// Marker for the designation `Text` node on the top cap. Driven by
/// [`update_hud`] — toggles between signal-cyan and alert-red.
#[derive(Component)]
struct DesignationText;

/// Identifies which HUD value cell a `Text` node is, so [`update_hud`]
/// can write the formatted heading / hull / condition string into the
/// right entity.
#[derive(Component, Copy, Clone, Debug, PartialEq, Eq)]
enum HudValue {
    Heading,
    Hull,
    Condition,
}

/// Marker for the root node of the in-game HUD strip.
/// Despawned when transitioning back to Lobby.
#[derive(Component)]
struct InGameHudStrip;

/// Marker for the root node of the lobby HUD strip.
/// Despawned when transitioning to InProgress.
#[derive(Component)]
struct LobbyHudStrip;

/// Identifies which lobby HUD value cell a `Text` node is.
#[derive(Component, Copy, Clone, Debug, PartialEq, Eq)]
enum LobbyHudValue {
    Clock,
    Players,
    Status,
}

// ── Red Alert vignette material ──────────────────────────────────────

/// `UiMaterial` driving the inset radial-gradient red vignette behind
/// the border. The single `intensity` uniform is in `[0.0, 1.0]`; the
/// shader fades the red glow from invisible at 0.0 to fully bright at
/// 1.0. Driven each frame by [`drive_vignette_intensity`] via the pure
/// helper [`pulse_intensity`].
///
/// The struct is padded to 16 bytes (4×f32) so the uniform buffer binding
/// satisfies `BUFFER_BINDINGS_NOT_16_BYTE_ALIGNED` requirements on
/// downlevel WebGL2 devices (integrated GPUs, SwiftShader in CI).
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct RedAlertVignetteMaterial {
    #[uniform(0)]
    pub intensity: f32,
    /// Padding — keeps the uniform block 16-byte aligned on downlevel WebGL2.
    #[uniform(0)]
    _pad0: f32,
    #[uniform(0)]
    _pad1: f32,
    #[uniform(0)]
    _pad2: f32,
}

impl UiMaterial for RedAlertVignetteMaterial {
    fn fragment_shader() -> ShaderRef {
        "shaders/red_alert_vignette.wgsl".into()
    }
}

// ── Plugin ───────────────────────────────────────────────────────────

/// Loads viewscreen border assets at startup, registers the Red Alert
/// vignette material, and renders the frame during `GameState::InProgress`.
pub struct ViewscreenBorderPlugin;

impl Plugin for ViewscreenBorderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiMaterialPlugin::<RedAlertVignetteMaterial>::default())
            .add_systems(Startup, (load_viewscreen_assets, spawn_border_on_startup, attach_initial_strip, spawn_lobby_screen).chain())
            .add_systems(
                Update,
                (
                    sync_hud_strips_to_phase,
                    swap_border_textures,
                    drive_vignette_intensity,
                    update_hud,
                    update_lobby_hud,
                    toggle_lobby_screen_visibility,
                    rebuild_lobby_station_grid,
                    update_lobby_header_values,
                ),
            );
    }
}

// ── Systems ──────────────────────────────────────────────────────────

fn load_viewscreen_assets(mut commands: Commands, asset_server: Res<AssetServer>) {
    let assets = ViewscreenAssets {
        corner_tl: asset_server.load("viewscreen/corner-tl.png"),
        corner_tr: asset_server.load("viewscreen/corner-tr.png"),
        corner_bl: asset_server.load("viewscreen/corner-bl.png"),
        corner_br: asset_server.load("viewscreen/corner-br.png"),
        edge_top: asset_server.load("viewscreen/edge-top.png"),
        edge_bottom: asset_server.load("viewscreen/edge-bottom.png"),
        edge_left: asset_server.load("viewscreen/edge-left.png"),
        edge_right: asset_server.load("viewscreen/edge-right.png"),
        cap_top: asset_server.load("viewscreen/cap-top.png"),
        cap_bottom: asset_server.load("viewscreen/cap-bottom.png"),
        corner_tl_alert: asset_server.load("viewscreen/corner-tl-alert.png"),
        corner_tr_alert: asset_server.load("viewscreen/corner-tr-alert.png"),
        corner_bl_alert: asset_server.load("viewscreen/corner-bl-alert.png"),
        corner_br_alert: asset_server.load("viewscreen/corner-br-alert.png"),
        edge_top_alert: asset_server.load("viewscreen/edge-top-alert.png"),
        edge_bottom_alert: asset_server.load("viewscreen/edge-bottom-alert.png"),
        edge_left_alert: asset_server.load("viewscreen/edge-left-alert.png"),
        edge_right_alert: asset_server.load("viewscreen/edge-right-alert.png"),
        cap_top_alert: asset_server.load("viewscreen/cap-top-alert.png"),
        cap_bottom_alert: asset_server.load("viewscreen/cap-bottom-alert.png"),
        font_display: asset_server.load("fonts/ChakraPetch-SemiBold.ttf"),
        font_mono: asset_server.load("fonts/JetBrainsMono-Regular.ttf"),
    };
    commands.insert_resource(assets);
}

/// Spawns the border frame (vignette, corners, edges, caps, designation).
fn spawn_border_on_startup(
    mut commands: Commands,
    assets: Res<ViewscreenAssets>,
    mut materials: ResMut<Assets<RedAlertVignetteMaterial>>,
) {
    let vignette = materials.add(RedAlertVignetteMaterial { intensity: 0.0, _pad0: 0.0, _pad1: 0.0, _pad2: 0.0 });
    commands.insert_resource(VignetteMaterialHandle(vignette.clone()));
    spawn_border_frame(&mut commands, &assets, vignette);
}

/// Startup system that runs after `spawn_border_on_startup` to attach the
/// initial lobby HUD strip as a child of the bottom cap entity.
fn attach_initial_strip(
    mut commands: Commands,
    assets: Res<ViewscreenAssets>,
    slots: Query<(&BorderSlot, Entity)>,
) {
    let Some(parent) = slots
        .iter()
        .find(|(slot, _)| **slot == BorderSlot::CapBottom)
        .map(|(_, e)| e)
    else {
        return;
    };
    let strip = spawn_lobby_hud_strip(&mut commands, &assets);
    commands.entity(parent).add_child(strip);
}

/// Swaps HUD strips on phase transition.
///
/// - `Lobby → InProgress`: despawn lobby strip, spawn in-game strip.
/// - `InProgress → Lobby`: despawn in-game strip, spawn lobby strip.
///
/// Idempotent — re-entering a phase while the correct strip already
/// exists is a no-op.
fn sync_hud_strips_to_phase(
    mut commands: Commands,
    state: Res<State<GamePhase>>,
    assets: Option<Res<ViewscreenAssets>>,
    slots: Query<(&BorderSlot, Entity)>,
    lobby_strip: Query<Entity, With<LobbyHudStrip>>,
    ingame_strip: Query<Entity, With<InGameHudStrip>>,
) {
    if !state.is_changed() {
        return;
    }
    let Some(assets) = assets else { return };
    let bottom_cap = slots
        .iter()
        .find(|(slot, _)| **slot == BorderSlot::CapBottom)
        .map(|(_, e)| e)
        .or_else(|| slots.iter().next().map(|(_, e)| e));

    match state.get() {
        GamePhase::InProgress => {
            for e in lobby_strip.iter() {
                commands.entity(e).despawn();
            }
            if ingame_strip.is_empty() {
                let strip = spawn_ingame_hud_strip(&mut commands, &assets);
                if let Some(parent) = bottom_cap {
                    commands.entity(parent).add_child(strip);
                }
            }
        }
        GamePhase::Lobby => {
            for e in ingame_strip.iter() {
                commands.entity(e).despawn();
            }
            if lobby_strip.is_empty() {
                let strip = spawn_lobby_hud_strip(&mut commands, &assets);
                if let Some(parent) = bottom_cap {
                    commands.entity(parent).add_child(strip);
                }
            }
        }
    }
}

/// Rewrites each border `ImageNode`'s `image` handle to the alert or
/// normal variant whenever `ShipState.red_alert` changes.
///
/// The swap is instant (one frame) — matches the demo's pop. The pulsing
/// vignette carries the temporal energy.
fn swap_border_textures(
    ship: Option<Res<ShipState>>,
    assets: Option<Res<ViewscreenAssets>>,
    mut q: Query<(&BorderSlot, &mut ImageNode)>,
) {
    let Some(ship) = ship else { return };
    let Some(assets) = assets else { return };
    if !ship.is_changed() {
        return;
    }
    let alert = ship.red_alert();
    for (slot, mut image_node) in q.iter_mut() {
        image_node.image = slot.handle(&assets, alert).clone();
    }
}

/// Per-frame system that drives the vignette material's `intensity`
/// uniform via the pure [`pulse_intensity`] helper.
fn drive_vignette_intensity(
    time: Res<Time>,
    ship: Option<Res<ShipState>>,
    handle: Option<Res<VignetteMaterialHandle>>,
    mut materials: ResMut<Assets<RedAlertVignetteMaterial>>,
) {
    let Some(ship) = ship else { return };
    let Some(handle) = handle else { return };
    let Some(material) = materials.get_mut(&handle.0) else { return };
    material.intensity = pulse_intensity(
        time.elapsed_secs(),
        ship.red_alert(),
        material.intensity,
        time.delta_secs(),
    );
}

// ── Pure helpers ─────────────────────────────────────────────────────

/// State-transition function for the Red Alert vignette intensity.
///
/// Combines a quarter-second on/off ease with a 1.3-second sine pulse
/// between [`MIN_INTENSITY`] and [`MAX_INTENSITY`] when active. Returns
/// `0.0` when fully off.
///
/// - `time_secs`: monotonic time (drives the sine phase).
/// - `red_alert`: the current alert state.
/// - `prev_intensity`: the previous frame's intensity (for ease ramping).
/// - `dt`: seconds since last frame.
///
/// Behaviour:
/// - `red_alert == false && prev_intensity == 0.0` → returns `0.0`.
/// - `red_alert == true` → ramps `prev_intensity` toward the current
///   sine target by at most `MAX_INTENSITY / EASE_DURATION * dt`, so the
///   rise from `0.0` to the band takes ~`EASE_DURATION` seconds.
/// - `red_alert == false && prev_intensity > 0.0` → ramps toward `0.0`
///   by the same per-second rate (~`EASE_DURATION` seconds to fully off).
///
/// Once `prev_intensity` is inside the pulse band, the sine target moves
/// strictly slower than the ease ramp, so the helper tracks the sine
/// directly — the band stays within `[MIN_INTENSITY, MAX_INTENSITY]`.
pub fn pulse_intensity(time_secs: f32, red_alert: bool, prev_intensity: f32, dt: f32) -> f32 {
    let max_step = (MAX_INTENSITY / EASE_DURATION) * dt;
    if red_alert {
        let target = sine_pulse(time_secs);
        approach(prev_intensity, target, max_step)
    } else if prev_intensity <= 0.0 {
        0.0
    } else {
        approach(prev_intensity, 0.0, max_step).max(0.0)
    }
}

/// Sine-wave value in `[MIN_INTENSITY, MAX_INTENSITY]` with period
/// [`PULSE_PERIOD`]. Phase 0 sits at the midpoint; the crest is at a
/// quarter period.
fn sine_pulse(time_secs: f32) -> f32 {
    let mid = (MIN_INTENSITY + MAX_INTENSITY) * 0.5;
    let amp = (MAX_INTENSITY - MIN_INTENSITY) * 0.5;
    mid + amp * (std::f32::consts::TAU * time_secs / PULSE_PERIOD).sin()
}

/// Move `current` toward `target` by at most `max_step` (always
/// non-negative). If the difference is already within `max_step`,
/// snap to `target`.
fn approach(current: f32, target: f32, max_step: f32) -> f32 {
    let delta = target - current;
    if delta.abs() <= max_step {
        target
    } else if delta > 0.0 {
        current + max_step
    } else {
        current - max_step
    }
}

/// Convert a ship yaw in radians to a 0–359 integer compass bearing.
///
/// `yaw == 0` means the ship faces forward; the bearing increases
/// clockwise as viewed from above. Negative yaw and multi-turn yaw
/// wrap correctly. The rounding boundary at 359.5° rounds up to 360
/// then wraps back to 0 (never returns 360).
pub fn yaw_to_compass_bearing(yaw_radians: f32) -> u32 {
    let degrees = yaw_radians.to_degrees().rem_euclid(360.0);
    (degrees.round() as u32) % 360
}

/// Per-frame system: update lobby HUD values (CLOCK / PLAYERS / STATUS).
/// Reads wall-clock time via `js_sys::Date`, player counts from
/// `Sessions` + `ShipStations`.
fn update_lobby_hud(
    sessions: Option<Res<Sessions>>,
    ship_stations: Option<Res<ShipStations>>,
    mut values: Query<(&LobbyHudValue, &mut Text)>,
) {
    if values.is_empty() {
        return;
    }

    // ── Clock ────────────────────────────────────────────────────────
    #[cfg(target_arch = "wasm32")]
    let clock_str = {
        let date = Date::new_0();
        format!(
            "{:02}:{:02}:{:02}",
            date.get_hours() as u32,
            date.get_minutes() as u32,
            date.get_seconds() as u32,
        )
    };
    #[cfg(not(target_arch = "wasm32"))]
    let clock_str = "--:--:--".to_string();

    // ── Players ──────────────────────────────────────────────────────
    let (connected, max) = if let (Some(sessions), Some(stations)) = (&sessions, &ship_stations) {
        let count = sessions
            .0
            .players()
            .iter()
            .filter(|p| p.connected && !p.consoles.is_empty())
            .count() as u32;
        (count, stations.max_players)
    } else {
        (0, 0)
    };
    let players_str = format!("{}/{}", connected, max);

    // ── Status ───────────────────────────────────────────────────────
    let status_str = if let Some(sessions) = &sessions {
        let all_have_console = sessions
            .0
            .players()
            .iter()
            .filter(|p| p.connected)
            .all(|p| !p.consoles.is_empty());
        let any_connected = sessions.0.players().iter().any(|p| p.connected);
        if any_connected && all_have_console {
            "READY FOR DEPARTURE"
        } else {
            "AWAITING CREW"
        }
    } else {
        "AWAITING CREW"
    };

    for (kind, mut text) in values.iter_mut() {
        let new_value = match kind {
            LobbyHudValue::Clock => clock_str.clone(),
            LobbyHudValue::Players => players_str.clone(),
            LobbyHudValue::Status => status_str.to_string(),
        };
        if text.0 != new_value {
            text.0 = new_value;
        }
    }
}

fn spawn_border_frame(
    commands: &mut Commands,
    assets: &ViewscreenAssets,
    vignette: Handle<RedAlertVignetteMaterial>,
) -> Entity {
    commands
        .spawn((
            ViewscreenBorderRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(0.0),
                left: Val::Px(0.0),
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                ..default()
            },
        ))
        .with_children(|parent| {
            // Spawn order determines z-order: later = on top.
            //
            // 1. Vignette (behind everything)
            // 2. Corners
            // 3. Edges (tiled)
            // 4. Caps (cover the centre seam where edge segments meet)
            // 5. Designation text (on top cap)

            // ── Vignette (spawned FIRST so border sprites occlude it) ─
            parent.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    ..default()
                },
                MaterialNode(vignette),
            ));

            // ── Corners ──────────────────────────────────────────────
            parent.spawn((
                BorderSlot::CornerTl,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Px(CORNER_W),
                    height: Val::Px(CORNER_H),
                    ..default()
                },
                ImageNode::new(assets.corner_tl.clone()),
            ));
            parent.spawn((
                BorderSlot::CornerTr,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    right: Val::Px(0.0),
                    width: Val::Px(CORNER_W),
                    height: Val::Px(CORNER_H),
                    ..default()
                },
                ImageNode::new(assets.corner_tr.clone()),
            ));
            parent.spawn((
                BorderSlot::CornerBl,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Px(CORNER_W),
                    height: Val::Px(CORNER_H),
                    ..default()
                },
                ImageNode::new(assets.corner_bl.clone()),
            ));
            parent.spawn((
                BorderSlot::CornerBr,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(0.0),
                    right: Val::Px(0.0),
                    width: Val::Px(CORNER_W),
                    height: Val::Px(CORNER_H),
                    ..default()
                },
                ImageNode::new(assets.corner_br.clone()),
            ));

            // ── Edges ────────────────────────────────────────────────
            //
            // Edges fill the gap between corners using `NodeImageMode::Tiled`.
            // The top/bottom edges are split into two segments (left-of-cap,
            // right-of-cap). Both segments share the same `BorderSlot` marker
            // so the swap system rewrites both in one query iteration.
            //
            // The caps are spawned AFTER the edges so they render on top,
            // covering the seam where the two edge segments meet.

            // Top edge — left segment (between TL corner and top cap).
            parent.spawn((
                BorderSlot::EdgeTop,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(CORNER_W),
                    right: Val::Percent(50.0),
                    margin: UiRect {
                        right: Val::Px(CAP_TOP_W / 2.0),
                        ..default()
                    },
                    height: Val::Px(EDGE_THICKNESS),
                    ..default()
                },
                ImageNode::new(assets.edge_top.clone())
                    .with_mode(NodeImageMode::Tiled {
                        tile_x: true,
                        tile_y: false,
                        stretch_value: 1.0,
                    }),
            ));
            // Top edge — right segment (between top cap and TR corner).
            parent.spawn((
                BorderSlot::EdgeTop,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Percent(50.0),
                    right: Val::Px(CORNER_W),
                    margin: UiRect {
                        left: Val::Px(CAP_TOP_W / 2.0),
                        ..default()
                    },
                    height: Val::Px(EDGE_THICKNESS),
                    ..default()
                },
                ImageNode::new(assets.edge_top.clone())
                    .with_mode(NodeImageMode::Tiled {
                        tile_x: true,
                        tile_y: false,
                        stretch_value: 1.0,
                    }),
            ));

            // Bottom edge — left segment.
            parent.spawn((
                BorderSlot::EdgeBottom,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(0.0),
                    left: Val::Px(CORNER_W),
                    right: Val::Percent(50.0),
                    margin: UiRect {
                        right: Val::Px(CAP_BOTTOM_W / 2.0),
                        ..default()
                    },
                    height: Val::Px(EDGE_THICKNESS),
                    ..default()
                },
                ImageNode::new(assets.edge_bottom.clone())
                    .with_mode(NodeImageMode::Tiled {
                        tile_x: true,
                        tile_y: false,
                        stretch_value: 1.0,
                    }),
            ));
            // Bottom edge — right segment.
            parent.spawn((
                BorderSlot::EdgeBottom,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(0.0),
                    left: Val::Percent(50.0),
                    right: Val::Px(CORNER_W),
                    margin: UiRect {
                        left: Val::Px(CAP_BOTTOM_W / 2.0),
                        ..default()
                    },
                    height: Val::Px(EDGE_THICKNESS),
                    ..default()
                },
                ImageNode::new(assets.edge_bottom.clone())
                    .with_mode(NodeImageMode::Tiled {
                        tile_x: true,
                        tile_y: false,
                        stretch_value: 1.0,
                    }),
            ));

            // Left edge — between TL and BL corners.
            parent.spawn((
                BorderSlot::EdgeLeft,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(0.0),
                    top: Val::Px(CORNER_H),
                    bottom: Val::Px(CORNER_H),
                    width: Val::Px(EDGE_THICKNESS),
                    ..default()
                },
                ImageNode::new(assets.edge_left.clone())
                    .with_mode(NodeImageMode::Tiled {
                        tile_x: false,
                        tile_y: true,
                        stretch_value: 1.0,
                    }),
            ));

            // Right edge — between TR and BR corners.
            parent.spawn((
                BorderSlot::EdgeRight,
                Node {
                    position_type: PositionType::Absolute,
                    right: Val::Px(0.0),
                    top: Val::Px(CORNER_H),
                    bottom: Val::Px(CORNER_H),
                    width: Val::Px(EDGE_THICKNESS),
                    ..default()
                },
                ImageNode::new(assets.edge_right.clone())
                    .with_mode(NodeImageMode::Tiled {
                        tile_x: false,
                        tile_y: true,
                        stretch_value: 1.0,
                    }),
            ));

            // ── Caps (spawned AFTER edges so they render on top) ────

            // Top cap (centred along top edge).
            parent.spawn((
                BorderSlot::CapTop,
                Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Percent(50.0),
                    margin: UiRect {
                        left: Val::Px(-CAP_TOP_W / 2.0),
                        ..default()
                    },
                    width: Val::Px(CAP_TOP_W),
                    height: Val::Px(CAP_H),
                    ..default()
                },
                ImageNode::new(assets.cap_top.clone()),
            ));

            // Bottom cap (centred along bottom edge).
            parent.spawn((
                BorderSlot::CapBottom,
                Node {
                    position_type: PositionType::Absolute,
                    bottom: Val::Px(0.0),
                    left: Val::Percent(50.0),
                    margin: UiRect {
                        left: Val::Px(-CAP_BOTTOM_W / 2.0),
                        ..default()
                    },
                    width: Val::Px(CAP_BOTTOM_W),
                    height: Val::Px(CAP_H),
                    ..default()
                },
                ImageNode::new(assets.cap_bottom.clone()),
            ));

            // ── Designation (centred on top cap) ─────────────────────
            parent
                .spawn(Node {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Percent(50.0),
                    width: Val::Px(CAP_TOP_W),
                    height: Val::Px(CAP_H),
                    margin: UiRect {
                        left: Val::Px(-CAP_TOP_W / 2.0),
                        ..default()
                    },
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    ..default()
                })
                .with_children(|d| {
                    d.spawn((
                        DesignationText,
                        Text::new(DESIGNATION_TEXT),
                        TextFont {
                            font: assets.font_display.clone(),
                            font_size: DESIGNATION_FONT_SIZE,
                            ..default()
                        },
                        TextColor(COLOR_SIGNAL_CYAN),
                    ));
                });

            // The bottom cap HUD strip is spawned separately as either
            // a lobby strip or an in-game strip, and swapped on phase
            // transition.  The initial lobby strip is added after the
            // border frame is spawned (see `spawn_border_on_startup`).
        })
        .id()
}

/// Spawn the lobby HUD strip (CLOCK / PLAYERS / STATUS) as an overlay
/// that fills the bottom cap. The strip should be attached as a child of
/// the bottom cap entity so it inherits the cap's position and size.
fn spawn_lobby_hud_strip(commands: &mut Commands, assets: &ViewscreenAssets) -> Entity {
    commands
        .spawn((
            LobbyHudStrip,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceAround,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|strip| {
            spawn_lobby_column(strip, assets, "CLOCK", "--:--:--", LobbyHudValue::Clock);
            spawn_lobby_column(strip, assets, "PLAYERS", "0/0", LobbyHudValue::Players);
            spawn_lobby_column(strip, assets, "STATUS", "AWAITING CREW", LobbyHudValue::Status);
        })
        .id()
}

/// Spawn the in-game HUD strip (HEADING / HULL / CONDITION) as an overlay
/// that fills the bottom cap. The strip should be attached as a child of
/// the bottom cap entity.
fn spawn_ingame_hud_strip(commands: &mut Commands, assets: &ViewscreenAssets) -> Entity {
    commands
        .spawn((
            InGameHudStrip,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                top: Val::Px(0.0),
                bottom: Val::Px(0.0),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceAround,
                align_items: AlignItems::Center,
                ..default()
            },
        ))
        .with_children(|strip| {
            spawn_status_column(strip, assets, "HEADING", "000", HudValue::Heading);
            spawn_status_column(strip, assets, "HULL", "100", HudValue::Hull);
            spawn_status_column(strip, assets, "CONDITION", "NOMINAL", HudValue::Condition);
        })
        .id()
}

/// Build one HEADING/HULL/CONDITION column inside the bottom-cap strip:
/// a Chakra Petch label above a JetBrains Mono value cell.
fn spawn_status_column(
    parent: &mut ChildSpawnerCommands,
    assets: &ViewscreenAssets,
    label: &str,
    initial_value: &str,
    value_kind: HudValue,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_children(|col| {
            col.spawn((
                Text::new(label),
                TextFont {
                    font: assets.font_display.clone(),
                    font_size: STATUS_LABEL_FONT_SIZE,
                    ..default()
                },
                TextColor(COLOR_NEUTRAL_LABEL),
            ));
            col.spawn((
                value_kind,
                Text::new(initial_value),
                TextFont {
                    font: assets.font_mono.clone(),
                    font_size: STATUS_VALUE_FONT_SIZE,
                    ..default()
                },
                TextColor(COLOR_SIGNAL_CYAN),
            ));
        });
}

/// Build one CLOCK/PLAYERS/STATUS column inside the lobby bottom-cap strip.
fn spawn_lobby_column(
    parent: &mut ChildSpawnerCommands,
    assets: &ViewscreenAssets,
    label: &str,
    initial_value: &str,
    value_kind: LobbyHudValue,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_children(|col| {
            col.spawn((
                Text::new(label),
                TextFont {
                    font: assets.font_display.clone(),
                    font_size: STATUS_LABEL_FONT_SIZE,
                    ..default()
                },
                TextColor(COLOR_NEUTRAL_LABEL),
            ));
            col.spawn((
                value_kind,
                Text::new(initial_value),
                TextFont {
                    font: assets.font_mono.clone(),
                    font_size: STATUS_VALUE_FONT_SIZE,
                    ..default()
                },
                TextColor(COLOR_SIGNAL_CYAN),
            ));
        });
}

/// Per-frame system: format heading / hull / condition strings, write
/// them into the value `Text` nodes, and toggle `TextColor` on the
/// designation and value cells between signal-cyan and alert-red.
///
/// Runs unconditionally — no change-detection plumbing. Per the PRD,
/// the cost (three `format!` calls + a few component writes) is
/// negligible and the simplicity is worth more than the saving.
fn update_hud(
    ship: Option<Res<ShipState>>,
    hull: Option<Res<ShipHullIntegrity>>,
    mut designation: Query<&mut TextColor, (With<DesignationText>, Without<HudValue>)>,
    mut values: Query<(&HudValue, &mut Text, &mut TextColor), Without<DesignationText>>,
) {
    let Some(ship) = ship else { return };
    let Some(hull) = hull else { return };
    let alert = ship.red_alert();
    let active_color = if alert { COLOR_ALERT_RED } else { COLOR_SIGNAL_CYAN };

    for mut color in designation.iter_mut() {
        if color.0 != active_color {
            *color = TextColor(active_color);
        }
    }

    for (kind, mut text, mut color) in values.iter_mut() {
        let new_value = match kind {
            HudValue::Heading => format!("{:03}", yaw_to_compass_bearing(ship.yaw)),
            HudValue::Hull => format!("{}", (hull.0.current().clamp(0.0, 100.0).round()) as i32),
            HudValue::Condition => if alert { "ALERT" } else { "NOMINAL" }.to_string(),
        };
        if text.0 != new_value {
            text.0 = new_value;
        }
        if color.0 != active_color {
            *color = TextColor(active_color);
        }
    }
}

// ── Lobby screen pure helpers ─────────────────────────────────────────

/// Returns the ready badge status string for the given all-claimed state.
pub(crate) fn ready_status(all_claimed: bool) -> &'static str {
    if all_claimed { "READY TO LAUNCH" } else { "AWAITING SELECTION" }
}

/// Returns the display label for a complexity preset name.
/// "Low" maps to "LOW"; everything else maps to "NORMAL".
pub(crate) fn complexity_label(preset: &str) -> &'static str {
    if preset == "Low" { "LOW" } else { "NORMAL" }
}

// ── Lobby screen systems ──────────────────────────────────────────────

/// Startup system — spawns the `LobbyScreenRoot` overlay with a header row
/// and an empty `LobbyGridRoot`. The grid is populated by
/// `rebuild_lobby_station_grid` on the first Update that sees valid state.
fn spawn_lobby_screen(mut commands: Commands, assets: Res<ViewscreenAssets>) {
    commands
        .spawn((
            LobbyScreenRoot,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(EDGE_THICKNESS),
                left: Val::Px(EDGE_THICKNESS),
                right: Val::Px(EDGE_THICKNESS),
                bottom: Val::Px(EDGE_THICKNESS),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(18.0)),
                row_gap: Val::Px(14.0),
                overflow: Overflow::clip(),
                ..default()
            },
        ))
        .with_children(|root| {
            // ── Header row ───────────────────────────────────────────
            root.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::FlexEnd,
                column_gap: Val::Px(20.0),
                width: Val::Percent(100.0),
                flex_shrink: 0.0,
                ..default()
            })
            .with_children(|head| {
                // Title block
                head.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(4.0),
                    flex_shrink: 0.0,
                    ..default()
                })
                .with_children(|tb| {
                    tb.spawn((
                        Text::new("// PRE-FLIGHT"),
                        TextFont {
                            font: assets.font_mono.clone(),
                            font_size: LOBBY_KICKER_SIZE,
                            ..default()
                        },
                        TextColor(COLOR_TEXT_DIM),
                    ));
                    tb.spawn((
                        Text::new("MISSION LOBBY"),
                        TextFont {
                            font: assets.font_display.clone(),
                            font_size: LOBBY_TITLE_SIZE,
                            ..default()
                        },
                        TextColor(COLOR_NEUTRAL_LABEL),
                    ));
                });

                // Horizontal rule (takes remaining horizontal space)
                head.spawn((
                    Node {
                        flex_grow: 1.0,
                        height: Val::Px(1.0),
                        margin: UiRect {
                            bottom: Val::Px(7.0),
                            ..default()
                        },
                        ..default()
                    },
                    BackgroundColor(COLOR_EDGE),
                ));

                // Crew counter + ready badge wrapper
                head.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(12.0),
                    align_items: AlignItems::Center,
                    flex_shrink: 0.0,
                    ..default()
                })
                .with_children(|right| {
                    // Crew counter box
                    right
                        .spawn((
                            Node {
                                flex_direction: FlexDirection::Row,
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(8.0),
                                padding: UiRect {
                                    left: Val::Px(14.0),
                                    right: Val::Px(14.0),
                                    top: Val::Px(8.0),
                                    bottom: Val::Px(8.0),
                                },
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(COLOR_GRAPHITE_DARK),
                            BorderColor::all(COLOR_EDGE),
                        ))
                        .with_children(|cc| {
                            cc.spawn((
                                Text::new("STATIONED"),
                                TextFont {
                                    font: assets.font_display.clone(),
                                    font_size: LOBBY_LABEL_SIZE,
                                    ..default()
                                },
                                TextColor(COLOR_TEXT_DIM),
                            ));
                            cc.spawn((
                                LobbyCrewDisplay,
                                Text::new("0/0"),
                                TextFont {
                                    font: assets.font_mono.clone(),
                                    font_size: 20.0,
                                    ..default()
                                },
                                TextColor(COLOR_SIGNAL_CYAN),
                            ));
                        });

                    // Ready badge
                    right
                        .spawn((
                            Node {
                                flex_direction: FlexDirection::Row,
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(10.0),
                                padding: UiRect {
                                    left: Val::Px(14.0),
                                    right: Val::Px(14.0),
                                    top: Val::Px(10.0),
                                    bottom: Val::Px(10.0),
                                },
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(COLOR_GRAPHITE_DARK),
                            BorderColor::all(COLOR_EDGE),
                        ))
                        .with_children(|badge| {
                            // LED dot (amber/gray until ready)
                            badge.spawn((
                                Node {
                                    width: Val::Px(10.0),
                                    height: Val::Px(10.0),
                                    flex_shrink: 0.0,
                                    ..default()
                                },
                                BackgroundColor(COLOR_EDGE),
                            ));
                            badge
                                .spawn(Node {
                                    flex_direction: FlexDirection::Column,
                                    row_gap: Val::Px(3.0),
                                    ..default()
                                })
                                .with_children(|txt| {
                                    txt.spawn((
                                        Text::new("SHIP STATUS"),
                                        TextFont {
                                            font: assets.font_display.clone(),
                                            font_size: LOBBY_LABEL_SIZE,
                                            ..default()
                                        },
                                        TextColor(COLOR_TEXT_DIM),
                                    ));
                                    txt.spawn((
                                        LobbyReadyVal,
                                        Text::new("AWAITING SELECTION"),
                                        TextFont {
                                            font: assets.font_display.clone(),
                                            font_size: LOBBY_VALUE_SIZE,
                                            ..default()
                                        },
                                        TextColor(COLOR_AMBER),
                                    ));
                                });
                        });
                });
            });

            // ── Station grid (fills remaining vertical space) ────────
            root.spawn((
                LobbyGridRoot,
                Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    flex_grow: 1.0,
                    align_content: AlignContent::FlexStart,
                    column_gap: Val::Px(12.0),
                    row_gap: Val::Px(12.0),
                    overflow: Overflow::clip(),
                    ..default()
                },
            ));
        });
}

/// Show the lobby overlay during Lobby phase; hide it during InProgress.
fn toggle_lobby_screen_visibility(
    state: Res<State<GamePhase>>,
    mut screen_q: Query<&mut Visibility, With<LobbyScreenRoot>>,
) {
    if !state.is_changed() {
        return;
    }
    let target = match state.get() {
        GamePhase::Lobby => Visibility::Visible,
        GamePhase::InProgress => Visibility::Hidden,
    };
    for mut vis in screen_q.iter_mut() {
        *vis = target;
    }
}

/// Clears and rebuilds the station card grid whenever session or complexity
/// state changes. The grid always has exactly 6 slots (3×2); active station
/// cards fill the front, empty "Reserved" placeholders fill the rest.
fn rebuild_lobby_station_grid(
    sessions: Option<Res<Sessions>>,
    ship_stations: Option<Res<ShipStations>>,
    complexity: Option<Res<ConsoleComplexityState>>,
    assets: Option<Res<ViewscreenAssets>>,
    state: Res<State<GamePhase>>,
    mut commands: Commands,
    grid_q: Query<Entity, With<LobbyGridRoot>>,
    old_cards: Query<Entity, With<LobbyStationCard>>,
) {
    let sessions_changed = sessions.as_ref().map(|s| s.is_changed()).unwrap_or(false);
    let stations_changed = ship_stations.as_ref().map(|s| s.is_changed()).unwrap_or(false);
    let complexity_changed = complexity.as_ref().map(|c| c.is_changed()).unwrap_or(false);

    if !sessions_changed && !stations_changed && !complexity_changed {
        return;
    }

    if state.get() != &GamePhase::Lobby {
        return;
    }

    let Some(assets) = assets else { return };
    let Ok(grid_entity) = grid_q.single() else { return };

    // Despawn all existing station cards
    for card_entity in old_cards.iter() {
        commands.entity(card_entity).despawn();
    }

    // Determine current player count and pick the station config
    let (station_defs, _player_count): (Vec<StationDef>, u32) =
        if let (Some(sessions), Some(stations)) = (&sessions, &ship_stations) {
            let count = sessions
                .0
                .players()
                .iter()
                .filter(|p| p.connected && !p.consoles.is_empty())
                .count() as u32;
            let clamped = count
                .max(stations.min_players)
                .min(stations.max_players);
            let defs = stations
                .configs
                .get(&clamped)
                .cloned()
                .unwrap_or_default();
            (defs, clamped)
        } else {
            (Vec::new(), 0)
        };

    let players = sessions
        .as_ref()
        .map(|s| s.0.players().to_vec())
        .unwrap_or_default();

    let default_complexity = ConsoleComplexityState::default();
    let complexity_state = complexity.as_deref().unwrap_or(&default_complexity);

    let active_count = station_defs.len();
    let mut new_cards: Vec<Entity> = Vec::new();

    for station_def in &station_defs {
        let occupant = players.iter().find(|p| {
            p.connected
                && !p.consoles.is_empty()
                && station_def
                    .consoles
                    .iter()
                    .all(|c| p.consoles.contains(c))
        });

        let cmplx_preset = station_def
            .consoles
            .first()
            .and_then(|c| complexity_state.presets.get(c).map(String::as_str))
            .unwrap_or("Std");
        let cmplx_str = complexity_label(cmplx_preset);
        let is_low = cmplx_preset == "Low";
        let claimed = occupant.is_some();

        let card = spawn_station_card(
            &mut commands,
            &assets,
            station_def,
            occupant.map(|p| p.name.as_str()),
            cmplx_str,
            is_low,
            claimed,
        );
        new_cards.push(card);
    }

    // Fill remaining slots with "Reserved" placeholders to always show 6 cells
    const MAX_GRID_SLOTS: usize = 6;
    for _ in active_count..MAX_GRID_SLOTS {
        let placeholder = spawn_station_placeholder(&mut commands, &assets, active_count);
        new_cards.push(placeholder);
    }

    for card in new_cards {
        commands.entity(grid_entity).add_child(card);
    }
}

/// Updates the crew counter and ready badge text whenever session counts change.
fn update_lobby_header_values(
    sessions: Option<Res<Sessions>>,
    ship_stations: Option<Res<ShipStations>>,
    mut crew_q: Query<&mut Text, (With<LobbyCrewDisplay>, Without<LobbyReadyVal>)>,
    mut ready_q: Query<&mut Text, (With<LobbyReadyVal>, Without<LobbyCrewDisplay>)>,
) {
    let sessions_changed = sessions.as_ref().map(|s| s.is_changed()).unwrap_or(false);
    let stations_changed = ship_stations.as_ref().map(|s| s.is_changed()).unwrap_or(false);
    if !sessions_changed && !stations_changed {
        return;
    }

    let (stationed, total, all_claimed) =
        if let (Some(sessions), Some(stations)) = (&sessions, &ship_stations) {
            let count = sessions
                .0
                .players()
                .iter()
                .filter(|p| p.connected && !p.consoles.is_empty())
                .count() as u32;
            let max = stations.max_players;
            (count, max, count >= max && max > 0)
        } else {
            (0, 0, false)
        };

    for mut text in crew_q.iter_mut() {
        let new_val = format!("{}/{}", stationed, total);
        if text.0 != new_val {
            text.0 = new_val;
        }
    }

    for mut text in ready_q.iter_mut() {
        let new_val = ready_status(all_claimed).to_string();
        if text.0 != new_val {
            text.0 = new_val;
        }
    }
}

// ── Lobby station card spawn helpers ─────────────────────────────────

/// Spawns a single station card for the 3×2 grid. Returns the root entity.
fn spawn_station_card(
    commands: &mut Commands,
    assets: &ViewscreenAssets,
    def: &StationDef,
    occupant_name: Option<&str>,
    cmplx_str: &str,
    is_low: bool,
    claimed: bool,
) -> Entity {
    let border_color = if claimed { COLOR_SIGNAL_CYAN } else { COLOR_EDGE };
    let name_color = if claimed { COLOR_SIGNAL_CYAN } else { COLOR_NEUTRAL_LABEL };
    let cmplx_color = if is_low { COLOR_SIGNAL_CYAN } else { COLOR_NEUTRAL_LABEL };
    let cmplx_border = if is_low { COLOR_SIGNAL_DEEP } else { COLOR_EDGE };

    let glyph_str = if !def.short_code.is_empty() {
        def.short_code.clone()
    } else {
        def.name
            .chars()
            .next()
            .map(|c| c.to_uppercase().to_string())
            .unwrap_or_else(|| "?".to_string())
    };

    let station_id = if !def.short_code.is_empty() {
        format!("STN-{}", def.short_code)
    } else {
        format!("STN-{}", def.name.to_uppercase().chars().take(3).collect::<String>())
    };

    commands
        .spawn((
            LobbyStationCard,
            Node {
                width: Val::Percent(31.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                padding: UiRect::all(Val::Px(12.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(COLOR_GRAPHITE_DARK),
            BorderColor::all(border_color),
        ))
        .with_children(|card| {
            // ── Card header: glyph | name | rank | LED ───────────────
            card.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|head| {
                // Short-code glyph box
                head.spawn((
                    Node {
                        width: Val::Px(28.0),
                        height: Val::Px(28.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(COLOR_GRAPHITE),
                    BorderColor::all(border_color),
                ))
                .with_children(|g| {
                    g.spawn((
                        Text::new(glyph_str),
                        TextFont {
                            font: assets.font_display.clone(),
                            font_size: 10.0,
                            ..default()
                        },
                        TextColor(name_color),
                    ));
                });

                // Station name
                head.spawn((
                    Node {
                        flex_grow: 1.0,
                        ..default()
                    },
                    Text::new(def.name.to_uppercase()),
                    TextFont {
                        font: assets.font_display.clone(),
                        font_size: LOBBY_CARD_NAME_SIZE,
                        ..default()
                    },
                    TextColor(name_color),
                ));

                // Rank badge
                head.spawn((
                    Node {
                        padding: UiRect {
                            left: Val::Px(5.0),
                            right: Val::Px(5.0),
                            top: Val::Px(3.0),
                            bottom: Val::Px(3.0),
                        },
                        border: UiRect::all(Val::Px(1.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.0)),
                    BorderColor::all(COLOR_EDGE),
                ))
                .with_children(|rb| {
                    rb.spawn((
                        Text::new(def.rank.clone()),
                        TextFont {
                            font: assets.font_mono.clone(),
                            font_size: LOBBY_CARD_SMALL_SIZE,
                            ..default()
                        },
                        TextColor(COLOR_TEXT_DIM),
                    ));
                });

                // LED dot
                head.spawn((
                    Node {
                        width: Val::Px(8.0),
                        height: Val::Px(8.0),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    BackgroundColor(if claimed { COLOR_SIGNAL_CYAN } else { COLOR_EDGE }),
                ));
            });

            // ── Description ──────────────────────────────────────────
            card.spawn((
                Text::new(def.description.clone()),
                TextFont {
                    font: assets.font_display.clone(),
                    font_size: 10.0,
                    ..default()
                },
                TextColor(COLOR_TEXT_DIM),
            ));

            // ── Consoles list ────────────────────────────────────────
            card.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(4.0),
                ..default()
            })
            .with_children(|cl| {
                cl.spawn((
                    Text::new(format!("CONSOLES · {}", def.consoles.len())),
                    TextFont {
                        font: assets.font_display.clone(),
                        font_size: LOBBY_CARD_SMALL_SIZE,
                        ..default()
                    },
                    TextColor(COLOR_TEXT_DIM),
                ));
                cl.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    flex_wrap: FlexWrap::Wrap,
                    column_gap: Val::Px(4.0),
                    row_gap: Val::Px(4.0),
                    ..default()
                })
                .with_children(|cr| {
                    for (i, console) in def.consoles.iter().enumerate() {
                        cr.spawn((
                            Node {
                                flex_direction: FlexDirection::Row,
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(4.0),
                                padding: UiRect {
                                    left: Val::Px(6.0),
                                    right: Val::Px(6.0),
                                    top: Val::Px(3.0),
                                    bottom: Val::Px(3.0),
                                },
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(COLOR_GRAPHITE),
                            BorderColor::all(COLOR_EDGE),
                        ))
                        .with_children(|cb| {
                            cb.spawn((
                                Text::new(format!("{:02}", i + 1)),
                                TextFont {
                                    font: assets.font_mono.clone(),
                                    font_size: LOBBY_CARD_SMALL_SIZE,
                                    ..default()
                                },
                                TextColor(if claimed {
                                    COLOR_SIGNAL_CYAN
                                } else {
                                    COLOR_TEXT_DIM
                                }),
                            ));
                            cb.spawn((
                                Text::new(console.display_name().to_uppercase()),
                                TextFont {
                                    font: assets.font_display.clone(),
                                    font_size: LOBBY_CARD_SMALL_SIZE,
                                    ..default()
                                },
                                TextColor(if claimed {
                                    COLOR_NEUTRAL_LABEL
                                } else {
                                    COLOR_TEXT_DIM
                                }),
                            ));
                        });
                    }
                });
            });

            // ── Occupant row (fills remaining vertical space) ────────
            let occupant_str = occupant_name.unwrap_or("— UNCLAIMED —");
            let initials: String = if let Some(name) = occupant_name {
                name.split(|c: char| !c.is_alphanumeric())
                    .filter(|s: &&str| !s.is_empty())
                    .take(2)
                    .filter_map(|s| s.chars().next())
                    .collect()
            } else {
                "—".to_string()
            };

            card.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    padding: UiRect {
                        left: Val::Px(8.0),
                        right: Val::Px(8.0),
                        top: Val::Px(5.0),
                        bottom: Val::Px(5.0),
                    },
                    border: UiRect::all(Val::Px(1.0)),
                    flex_grow: 1.0,
                    ..default()
                },
                BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.35)),
                BorderColor::all(COLOR_EDGE),
            ))
            .with_children(|occ| {
                // Avatar initials box
                occ.spawn((
                    Node {
                        width: Val::Px(22.0),
                        height: Val::Px(22.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(1.0)),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    BackgroundColor(COLOR_GRAPHITE),
                    BorderColor::all(if claimed { COLOR_SIGNAL_CYAN } else { COLOR_EDGE }),
                ))
                .with_children(|av| {
                    av.spawn((
                        Text::new(initials),
                        TextFont {
                            font: assets.font_mono.clone(),
                            font_size: 9.0,
                            ..default()
                        },
                        TextColor(if claimed { COLOR_SIGNAL_CYAN } else { COLOR_TEXT_DIM }),
                    ));
                });

                // Occupant name
                occ.spawn((
                    Node {
                        flex_grow: 1.0,
                        overflow: Overflow::clip(),
                        ..default()
                    },
                    Text::new(occupant_str.to_uppercase()),
                    TextFont {
                        font: assets.font_display.clone(),
                        font_size: 11.0,
                        ..default()
                    },
                    TextColor(if claimed { COLOR_NEUTRAL_LABEL } else { COLOR_TEXT_DIM }),
                ));
            });

            // ── Footer: complexity pill + station ID ─────────────────
            card.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                padding: UiRect {
                    top: Val::Px(6.0),
                    ..default()
                },
                flex_shrink: 0.0,
                ..default()
            })
            .with_children(|foot| {
                // Complexity label + pill
                foot.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|cmplx| {
                    cmplx.spawn((
                        Text::new("COMPLEXITY"),
                        TextFont {
                            font: assets.font_display.clone(),
                            font_size: LOBBY_CARD_SMALL_SIZE,
                            ..default()
                        },
                        TextColor(COLOR_TEXT_DIM),
                    ));
                    cmplx
                        .spawn((
                            Node {
                                padding: UiRect {
                                    left: Val::Px(7.0),
                                    right: Val::Px(7.0),
                                    top: Val::Px(3.0),
                                    bottom: Val::Px(3.0),
                                },
                                border: UiRect::all(Val::Px(1.0)),
                                ..default()
                            },
                            BackgroundColor(COLOR_GRAPHITE),
                            BorderColor::all(cmplx_border),
                        ))
                        .with_children(|pill| {
                            pill.spawn((
                                Text::new(cmplx_str),
                                TextFont {
                                    font: assets.font_display.clone(),
                                    font_size: LOBBY_CARD_SMALL_SIZE,
                                    ..default()
                                },
                                TextColor(cmplx_color),
                            ));
                        });
                });

                // Station ID
                foot.spawn((
                    Text::new(station_id),
                    TextFont {
                        font: assets.font_mono.clone(),
                        font_size: LOBBY_CARD_SMALL_SIZE,
                        ..default()
                    },
                    TextColor(COLOR_TEXT_DIM),
                ));
            });
        })
        .id()
}

/// Spawns an empty "Reserved" placeholder slot for unused grid positions.
fn spawn_station_placeholder(
    commands: &mut Commands,
    assets: &ViewscreenAssets,
    active_count: usize,
) -> Entity {
    commands
        .spawn((
            LobbyStationCard,
            Node {
                width: Val::Percent(31.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(5.0),
                padding: UiRect::all(Val::Px(14.0)),
                border: UiRect::all(Val::Px(1.0)),
                ..default()
            },
            BackgroundColor(Color::srgba(0.022, 0.024, 0.028, 1.0)),
            BorderColor::all(Color::srgba(0.227, 0.251, 0.286, 0.4)),
        ))
        .with_children(|p| {
            p.spawn((
                Text::new("RESERVED"),
                TextFont {
                    font: assets.font_display.clone(),
                    font_size: LOBBY_CARD_SMALL_SIZE,
                    ..default()
                },
                TextColor(COLOR_TEXT_DIM),
            ));
            p.spawn((
                Text::new(format!("Inactive at {}P", active_count)),
                TextFont {
                    font: assets.font_mono.clone(),
                    font_size: LOBBY_CARD_SMALL_SIZE,
                    ..default()
                },
                TextColor(COLOR_TEXT_DIM),
            ));
        })
        .id()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Per-frame `dt` at 60 Hz; matches the typical render cadence.
    const DT_60HZ: f32 = 1.0 / 60.0;

    /// Per-frame ramp ceiling used internally by `pulse_intensity` —
    /// kept in sync with the implementation so tests can reason about
    /// the ease window without hard-coding magic numbers.
    fn max_step_per_frame() -> f32 {
        (MAX_INTENSITY / EASE_DURATION) * DT_60HZ
    }

    #[test]
    fn idle_stays_at_zero() {
        // red_alert=false, prev=0 → exactly 0, no drift.
        let out = pulse_intensity(0.0, false, 0.0, DT_60HZ);
        assert_eq!(out, 0.0);

        // Time advancing while idle never lifts off zero.
        let out = pulse_intensity(123.4, false, 0.0, DT_60HZ);
        assert_eq!(out, 0.0);
    }

    #[test]
    fn alert_on_rises_monotonically_during_ease_window() {
        // From a cold start, simulate the quarter-second ease at 60 Hz
        // and confirm the intensity only ever goes up until it reaches
        // the pulse band.
        let mut prev = 0.0;
        let mut t = 0.0;
        let frames_in_ease = (EASE_DURATION / DT_60HZ).ceil() as usize;
        for _ in 0..frames_in_ease {
            let next = pulse_intensity(t, true, prev, DT_60HZ);
            assert!(
                next >= prev - 1e-6,
                "intensity decreased during alert-on ease (prev={prev}, next={next})"
            );
            prev = next;
            t += DT_60HZ;
        }
        // After the ease window, the intensity should have reached at
        // least MIN_INTENSITY (i.e. it has caught up with the pulse band).
        assert!(
            prev >= MIN_INTENSITY - 1e-3,
            "intensity {prev} did not reach pulse band after {EASE_DURATION}s ease"
        );
    }

    #[test]
    fn alert_off_decays_smoothly_to_zero_within_ease_window() {
        // Start at peak intensity, toggle off, simulate forward —
        // the intensity must fall monotonically and hit exactly 0.0
        // within the ease window (plus a small slack frame).
        let mut prev = MAX_INTENSITY;
        let mut t = 0.0;
        let frames = (EASE_DURATION / DT_60HZ).ceil() as usize + 2;
        let mut hit_zero = false;
        for _ in 0..frames {
            let next = pulse_intensity(t, false, prev, DT_60HZ);
            assert!(
                next <= prev + 1e-6,
                "intensity increased during alert-off decay (prev={prev}, next={next})"
            );
            assert!(next >= 0.0, "intensity went negative");
            if next == 0.0 {
                hit_zero = true;
            }
            prev = next;
            t += DT_60HZ;
        }
        assert!(hit_zero, "intensity did not reach 0 within {EASE_DURATION}s ease");

        // Once at zero, further idle frames must stay at zero.
        let next = pulse_intensity(t, false, 0.0, DT_60HZ);
        assert_eq!(next, 0.0);
    }

    #[test]
    fn steady_state_pulse_stays_within_band() {
        // After the ease window, simulate a few full pulse periods and
        // confirm the intensity stays inside [MIN_INTENSITY, MAX_INTENSITY]
        // within a small numeric slack.
        let mut prev = MIN_INTENSITY; // pretend we already eased in
        let mut t = 0.0;
        let frames = ((PULSE_PERIOD * 3.0) / DT_60HZ).ceil() as usize;
        let mut min_seen = f32::INFINITY;
        let mut max_seen = f32::NEG_INFINITY;
        for _ in 0..frames {
            let next = pulse_intensity(t, true, prev, DT_60HZ);
            min_seen = min_seen.min(next);
            max_seen = max_seen.max(next);
            prev = next;
            t += DT_60HZ;
        }
        let slack = max_step_per_frame() + 1e-3;
        assert!(
            min_seen >= MIN_INTENSITY - slack,
            "min {min_seen} below band lower bound {MIN_INTENSITY}"
        );
        assert!(
            max_seen <= MAX_INTENSITY + 1e-3,
            "max {max_seen} above band upper bound {MAX_INTENSITY}"
        );
    }

    #[test]
    fn sine_phase_points_match_target_band() {
        // sine_pulse(t) at t=0 should be the band midpoint.
        let mid = (MIN_INTENSITY + MAX_INTENSITY) * 0.5;
        assert!((sine_pulse(0.0) - mid).abs() < 1e-5);

        // Quarter period → crest (MAX_INTENSITY).
        let quarter = PULSE_PERIOD / 4.0;
        assert!((sine_pulse(quarter) - MAX_INTENSITY).abs() < 1e-5);

        // Half period → midpoint again (descending).
        let half = PULSE_PERIOD / 2.0;
        assert!((sine_pulse(half) - mid).abs() < 1e-5);

        // Three-quarter period → trough (MIN_INTENSITY).
        let three_quarter = 3.0 * PULSE_PERIOD / 4.0;
        assert!((sine_pulse(three_quarter) - MIN_INTENSITY).abs() < 1e-5);

        // Full period → back to midpoint.
        assert!((sine_pulse(PULSE_PERIOD) - mid).abs() < 1e-5);
    }

    #[test]
    fn approach_snaps_when_within_step() {
        // delta smaller than max_step → returns target exactly.
        assert_eq!(approach(0.5, 0.51, 0.1), 0.51);
        assert_eq!(approach(0.5, 0.49, 0.1), 0.49);
    }

    #[test]
    fn approach_steps_toward_target_when_outside_step() {
        // Larger delta → moves by exactly max_step.
        assert!((approach(0.0, 1.0, 0.25) - 0.25).abs() < 1e-6);
        assert!((approach(1.0, 0.0, 0.25) - 0.75).abs() < 1e-6);
    }

    // ── yaw_to_compass_bearing ───────────────────────────────────────

    #[test]
    fn bearing_zero_yaw_is_zero() {
        assert_eq!(yaw_to_compass_bearing(0.0), 0);
    }

    #[test]
    fn bearing_quarter_turn_is_ninety() {
        assert_eq!(yaw_to_compass_bearing(std::f32::consts::FRAC_PI_2), 90);
    }

    #[test]
    fn bearing_half_turn_is_one_eighty() {
        assert_eq!(yaw_to_compass_bearing(std::f32::consts::PI), 180);
    }

    #[test]
    fn bearing_three_quarter_turn_is_two_seventy() {
        assert_eq!(yaw_to_compass_bearing(3.0 * std::f32::consts::FRAC_PI_2), 270);
    }

    #[test]
    fn bearing_full_turn_wraps_to_zero() {
        assert_eq!(yaw_to_compass_bearing(std::f32::consts::TAU), 0);
    }

    #[test]
    fn bearing_negative_yaw_wraps_positive() {
        // -π/2 rad = -90° → 270°
        assert_eq!(yaw_to_compass_bearing(-std::f32::consts::FRAC_PI_2), 270);
    }

    #[test]
    fn bearing_multi_turn_yaw_wraps() {
        // 2.5 turns: 2τ + π/2 → 90°
        let yaw = 2.0 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2;
        assert_eq!(yaw_to_compass_bearing(yaw), 90);
    }

    #[test]
    fn bearing_rounds_359_5_to_zero_not_360() {
        // 359.5° rounds to 360 then wraps to 0.
        let yaw = 359.5_f32.to_radians();
        assert_eq!(yaw_to_compass_bearing(yaw), 0);
    }

    // ── ready_status ─────────────────────────────────────────────────

    #[test]
    fn ready_status_awaiting_when_not_all_claimed() {
        assert_eq!(ready_status(false), "AWAITING SELECTION");
    }

    #[test]
    fn ready_status_ready_when_all_claimed() {
        assert_eq!(ready_status(true), "READY TO LAUNCH");
    }

    // ── complexity_label ─────────────────────────────────────────────

    #[test]
    fn complexity_label_low_for_low_preset() {
        assert_eq!(complexity_label("Low"), "LOW");
    }

    #[test]
    fn complexity_label_normal_for_std_preset() {
        assert_eq!(complexity_label("Std"), "NORMAL");
    }

    #[test]
    fn complexity_label_normal_for_unknown_preset() {
        assert_eq!(complexity_label("whatevs"), "NORMAL");
    }

    #[test]
    fn slot_handle_picks_normal_or_alert_variant() {
        // Sanity check that BorderSlot::handle returns distinct asset
        // ids for the normal and alert variants of one slot.
        let assets = test_assets();
        let normal = BorderSlot::CornerTl.handle(&assets, false);
        let alert = BorderSlot::CornerTl.handle(&assets, true);
        assert_ne!(normal.id(), alert.id());
    }

    fn test_assets() -> ViewscreenAssets {
        // Construct dummy handles — none of the fields are dereffed in
        // these tests, we only compare `Handle::id()`. In Bevy 0.18 a
        // weak handle is built from a `Uuid` via `Handle::from(uuid)`.
        use bevy::asset::uuid::Uuid;
        let h = |n: u128| -> Handle<Image> { Uuid::from_u128(n).into() };
        let f = |n: u128| -> Handle<Font> { Uuid::from_u128(n).into() };
        ViewscreenAssets {
            corner_tl: h(1),
            corner_tr: h(2),
            corner_bl: h(3),
            corner_br: h(4),
            edge_top: h(5),
            edge_bottom: h(6),
            edge_left: h(7),
            edge_right: h(8),
            cap_top: h(9),
            cap_bottom: h(10),
            corner_tl_alert: h(11),
            corner_tr_alert: h(12),
            corner_bl_alert: h(13),
            corner_br_alert: h(14),
            edge_top_alert: h(15),
            edge_bottom_alert: h(16),
            edge_left_alert: h(17),
            edge_right_alert: h(18),
            cap_top_alert: h(19),
            cap_bottom_alert: h(20),
            font_display: f(21),
            font_mono: f(22),
        }
    }
}
