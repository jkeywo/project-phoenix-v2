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
//! - **Lobby strip** (spawned at startup): CLOCK / PLAYERS / STATUS.
//!   CLOCK shows the real-world `hh:mm:ss` via `js_sys::Date`.
//!   PLAYERS shows `connected / max_players` from `SessionManager` and
//!   `ShipStations`. STATUS shows `AWAITING CREW` when any connected
//!   player has no console, `READY FOR DEPARTURE` when all do.
//! - On `InProgress` the lobby strip is despawned; the in-game
//!   HEADING / HULL / CONDITION readout is now rendered by the HTML
//!   viewscreen overlay (issue #422), fed by `HudStateChanged` messages.
//!
//! ## Red Alert
//!
//! When `ShipState.red_alert` flips:
//!
//! - Each border `ImageNode`'s texture handle is swapped instantly
//!   between its normal and alert variant by [`swap_border_textures`].
//! - The full-screen red vignette pulse is owned by the HTML viewscreen
//!   overlay's CSS (issue #422), driven by the `red_alert` field of the
//!   `ViewscreenHudState` pushed to JS. The shared
//!   [`RedAlertVignetteMaterial`] is kept only for the shield-hit white
//!   flash; its red `intensity` uniform is held at `0.0`.

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::ui_render::prelude::{UiMaterial, UiMaterialPlugin};
#[cfg(target_arch = "wasm32")]
use js_sys::Date;

use rand::Rng;

use crate::codec;
use crate::console_ai_plugin::ConsoleComplexityState;
use crate::lobby::{OutboundMessage, Sessions, WorldResource};
use crate::console_bridge::{HudStateChanged, LobbyStateChanged};
use crate::messages::{GamePhase, LobbyStatePayload, ServerMessage, StationPayload, ViewscreenHudState};
use crate::server::renderer::GameCamera;
use crate::ship_state::ShipState;
use crate::sim_sets::SimSet;
use crate::simulation::ShipHullIntegrity;
use crate::stations_config::ShipStations;

// ── Layout constants ─────────────────────────────────────────────────
//

// ── Shield flash constants ────────────────────────────────────────────

/// Rate at which shield-hit flash decays per second (1.0 → 0.0 in 0.3 s).
const FLASH_DECAY_RATE: f32 = 1.0 / 0.3;

/// Exponential decay rate for hull-damage camera shake.
/// `exp(-5.0 * 0.8) ≈ 0.018` — fully settled within ~0.8 s at max magnitude.
const SHAKE_DECAY_RATE: f32 = 5.0;

// ── HUD constants ────────────────────────────────────────────────────

/// Signal-cyan `#5fd8e8` — designation + status values when nominal.
const COLOR_SIGNAL_CYAN: Color = Color::srgb(0.373, 0.847, 0.910);

/// Neutral `#b8c0c8` — status labels (never swap colour).
const COLOR_NEUTRAL_LABEL: Color = Color::srgb(0.722, 0.753, 0.784);


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

/// Tracks the shield-hit white flash overlay on the viewscreen.
///
/// `intensity` is set by [`process_shield_flash`] when a `DamageTaken`
/// with `shield > 0` arrives, then decayed toward zero by
/// [`drive_vignette_intensity`] at [`FLASH_DECAY_RATE`] per second.
#[derive(Resource, Default)]
pub struct ShieldFlashState {
    /// Current flash intensity (0.0 = no flash, 1.0 = full white).
    pub intensity: f32,
}

/// Tracks hull-damage camera shake on the viewscreen.
///
/// `magnitude` is accumulated by [`process_hull_shake`] on each
/// `DamageTaken` with `hull > 0`, then decayed exponentially each
/// frame by [`apply_camera_shake`] — fully settled in ~0.8 s at max.
#[derive(Resource, Default)]
pub struct ShakeState {
    /// Current shake magnitude in world units (0.0 = no shake).
    pub magnitude: f32,
}

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

// ── HUD marker components ────────────────────────────────────────────


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

/// `UiMaterial` behind the border. The `intensity` uniform (red vignette) is
/// held at `0.0` — red alert is now drawn by the HTML overlay's CSS (issue
/// #422). The `flash_intensity` uniform is still driven each frame by
/// [`drive_vignette_intensity`] for the shield-hit white flash.
///
/// The struct is padded to 16 bytes (4×f32) so the uniform buffer binding
/// satisfies `BUFFER_BINDINGS_NOT_16_BYTE_ALIGNED` requirements on
/// downlevel WebGL2 devices (integrated GPUs, SwiftShader in CI).
#[derive(AsBindGroup, Asset, TypePath, Debug, Clone)]
pub struct RedAlertVignetteMaterial {
    #[uniform(0)]
    pub intensity: f32,
    /// Flash intensity (0–1) for shield-hit white overlay.
    /// Set by the shield-flash system, decayed each frame.
    #[uniform(0)]
    pub flash_intensity: f32,
    /// Aspect ratio (width / height) of the viewport. Used by the shader
    /// to compute edge distances in pixel-space so the vignette has
    /// uniform thickness on all four edges regardless of screen shape.
    #[uniform(0)]
    pub aspect_ratio: f32,
    /// Padding — keeps the uniform block 16-byte aligned on downlevel WebGL2.
    #[uniform(0)]
    _pad0: f32,
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
            .add_message::<HudStateChanged>()
            .add_message::<LobbyStateChanged>()
            .init_resource::<ShieldFlashState>()
            .init_resource::<ShakeState>()
            // The viewscreen border, HUD state, and red-alert vignette are managed here.
            // The lobby UI is rendered entirely by the HTML overlay in `server.html`
            // (`window.__updateLobby`); this plugin pushes `LobbyStatePayload` snapshots
            // via `push_lobby_state`. The Bevy `LobbyScreenRoot` tree was deleted as part
            // of issue #436's HTML rebuild — see wiki/concepts/server-lobby-ui.md.
            .add_systems(Startup, (load_viewscreen_assets, spawn_border_on_startup, spawn_hud_state_entity).chain())
            .add_systems(
                Update,
                (
                    sync_hud_strips_to_phase,
                    swap_border_textures,
                    process_shield_flash.after(SimSet::Broadcast),
                    drive_vignette_intensity.after(process_shield_flash),
                    process_hull_shake.after(SimSet::Broadcast),
                    apply_camera_shake
                        .after(process_shield_flash)
                        .after(process_hull_shake)
                        .run_if(in_state(GamePhase::InProgress)),
                    update_lobby_hud,
                    push_lobby_state,
                ),
            )
            // HUD overlay reflects in-game readouts (heading / hull / condition);
            // only push while InProgress so the lobby phase emits no HUD state.
            .add_systems(
                Update,
                (
                    recompute_hud_state,
                    push_hud_state.after(recompute_hud_state),
                )
                    .run_if(in_state(GamePhase::InProgress)),
            );
    }
}

/// Reads server lobby resources and emits `LobbyStateChanged` for the HTML
/// lobby overlay whenever the state changes. Runs in `Update` so the bridge's
/// `flush_lobby_state` (in `PostUpdate`) forwards it to JS.
fn push_lobby_state(
    sessions: Option<Res<Sessions>>,
    ship_stations: Option<Res<ShipStations>>,
    phase: Res<State<GamePhase>>,
    complexity: Option<Res<ConsoleComplexityState>>,
    world_resource: Option<Res<WorldResource>>,
    mut writer: MessageWriter<LobbyStateChanged>,
) {
    let Some(sessions) = sessions else { return };
    let Some(stations) = ship_stations else { return };
    let Some(complexity) = complexity else { return };

    let players = sessions.0.players();
    let connected_count = players.iter().filter(|p| p.connected).count() as u32;
    let display_count = connected_count.max(stations.min_players).min(stations.max_players);

    let mut station_payloads: Vec<StationPayload> = Vec::new();
    let mut spectators: Vec<String> = Vec::new();

    if let Some(defs) = stations.configs.get(&display_count) {
        for def in defs {
            let holder = players.iter().find(|p| {
                p.connected && !p.consoles.is_empty()
                    && def.consoles.iter().all(|c| p.consoles.contains(c))
            });
            let preset_names: Vec<String> = def.consoles.iter()
                .map(|c| complexity.presets.get(c).map(String::as_str).unwrap_or("Std").to_string())
                .collect();
            station_payloads.push(StationPayload {
                name: def.name.clone(),
                short_code: def.short_code.clone(),
                rank: def.rank.clone(),
                consoles: def.consoles.clone(),
                holder_name: holder.map(|p| p.name.clone()),
                is_mine: false,
                preset_names,
            });
        }
    }

    // Players with no consoles who are connected are spectators
    // (only when connected count exceeds max_players).
    if stations.max_players > 0 && connected_count > stations.max_players {
        for p in players.iter().filter(|p| p.connected && p.consoles.is_empty()) {
            spectators.push(p.name.clone());
        }
    }

    let all_held: Vec<_> = players.iter().flat_map(|p| p.consoles.iter().cloned()).collect();
    let all_filled = crate::stations_config::all_stations_filled(&stations, display_count, &all_held);

    let scenario_title = world_resource.as_ref()
        .map(|w| w.0.scenario_title.clone())
        .unwrap_or_default();

    let scenario_body = world_resource.as_ref()
        .map(|w| w.0.scenario_description.clone())
        .unwrap_or_default();

    let payload = LobbyStatePayload {
        phase: format!("{:?}", phase.get()),
        scenario_title,
        scenario_body,
        crew_count: station_payloads.iter().filter(|s| s.holder_name.is_some()).count() as u32,
        max_players: stations.max_players,
        all_stations_filled: all_filled,
        stations: station_payloads,
        spectators,
    };

    if let Ok(json) = codec::encode_lobby_state(&payload) {
        writer.write(LobbyStateChanged { json });
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
    window: Query<&Window>,
    mut materials: ResMut<Assets<RedAlertVignetteMaterial>>,
) {
    let aspect_ratio = window
        .iter()
        .next()
        .map(|w| (w.width() / w.height()).max(0.01))
        .unwrap_or(1.0);
    let vignette = materials.add(RedAlertVignetteMaterial {
        intensity: 0.0,
        flash_intensity: 0.0,
        aspect_ratio,
        _pad0: 0.0,
    });
    commands.insert_resource(VignetteMaterialHandle(vignette.clone()));
    // Border frame and lobby UI are owned by the HTML overlay in `server.html`
    // (issue #436 deleted the Bevy `LobbyScreenRoot` tree; `spawn_border_frame`
    // is kept for now but not called). The vignette material is still created
    // above for the shield-hit white flash.
    let _ = &assets; // suppress unused-variable warning
}


/// Swaps the lobby HUD strip on phase transition.
///
/// The in-game HEADING/HULL/CONDITION strip is now owned by the HTML
/// viewscreen overlay (issue #422), so on `InProgress` we simply despawn the
/// lobby strip and spawn nothing in-game. On `Lobby` the lobby strip is
/// (re)spawned.
///
/// Idempotent — re-entering a phase while the correct strip already exists is
/// a no-op.
fn sync_hud_strips_to_phase(
    mut commands: Commands,
    state: Res<State<GamePhase>>,
    assets: Option<Res<ViewscreenAssets>>,
    slots: Query<(&BorderSlot, Entity)>,
    lobby_strip: Query<Entity, With<LobbyHudStrip>>,
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
            // In-game HUD moved to HTML — just tear down the lobby strip.
            for e in lobby_strip.iter() {
                commands.entity(e).despawn();
            }
        }
        GamePhase::Lobby => {
            // Only spawn the Bevy lobby strip when the border frame exists (i.e.
            // `bottom_cap` is Some). When the HTML overlay owns the border
            // (issue #422), `bottom_cap` is None and we skip the Bevy strip so
            // no stray root node appears on screen.
            if lobby_strip.is_empty() {
                if let Some(parent) = bottom_cap {
                    let strip = spawn_lobby_hud_strip(&mut commands, &assets);
                    commands.entity(parent).add_child(strip);
                }
            }
        }
        GamePhase::GameOver => {
            // Keep the current HUD strip visible during game-over.
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

/// Reads [`OutboundMessage`] for [`ServerMessage::DamageTaken`] with
/// `shield > 0` and sets [`ShieldFlashState::intensity`] scaled linearly
/// over 0–30 HP absorbed (full white at 30+).
///
/// Runs after `SimSet::Broadcast` so the outbox has been drained into
/// `OutboundMessage` messages and is safe to read.
fn process_shield_flash(
    mut outbound: MessageReader<OutboundMessage>,
    mut flash: ResMut<ShieldFlashState>,
) {
    for msg in outbound.read() {
        if let ServerMessage::DamageTaken { shield, .. } = &msg.msg {
            if *shield > 0.0 {
                flash.intensity = (*shield / 30.0).min(1.0);
            }
        }
    }
}

/// Reads [`OutboundMessage`] for [`ServerMessage::DamageTaken`] with
/// `hull > 0` and accumulates [`ShakeState::magnitude`] scaled linearly
/// over 0–30 HP (max ~5 world units at 30+ HP).
///
/// Runs after `SimSet::Broadcast` so the outbox has been drained into
/// `OutboundMessage` messages and is safe to read.
fn process_hull_shake(
    mut outbound: MessageReader<OutboundMessage>,
    mut shake: ResMut<ShakeState>,
) {
    for msg in outbound.read() {
        if let ServerMessage::DamageTaken { hull, .. } = &msg.msg {
            if *hull > 0.0 {
                let added = (*hull / 30.0).min(1.0) * 5.0;
                shake.magnitude += added;
            }
        }
    }
}

/// Applies a random XZ offset to the active 3D camera each frame based on
/// [`ShakeState::magnitude`], then decays the magnitude exponentially.
///
/// Runs after [`process_hull_shake`] (so it reads the accumulated shake
/// for the current frame) and after [`hull_camera`] in the renderer plugin
/// (so the base camera position is already set).
///
/// When magnitude drops below 0.01 units the shake is fully settled; the
/// camera is left at the position set by [`hull_camera`].
fn apply_camera_shake(
    time: Res<Time>,
    mut shake: ResMut<ShakeState>,
    mut cam_query: Query<&mut Transform, With<GameCamera>>,
) {
    let Ok(mut transform) = cam_query.single_mut() else { return };

    if shake.magnitude > 0.01 {
        let dt = time.delta_secs();
        let mut rng = rand::rng();
        let offset_x = rng.random_range(-shake.magnitude..shake.magnitude);
        let offset_z = rng.random_range(-shake.magnitude..shake.magnitude);

        transform.translation.x += offset_x;
        transform.translation.z += offset_z;

        shake.magnitude *= (-SHAKE_DECAY_RATE * dt).exp();
        if shake.magnitude < 0.01 {
            shake.magnitude = 0.0;
        }
    }
}

/// Per-frame system that decays the shield-hit white flash toward zero and
/// applies it to the vignette material.
///
/// Red alert is now owned by the HTML CSS vignette (issue #422), so the
/// material's `intensity` uniform is held at `0.0` — only the shield-flash
/// path still drives this shared material.
fn drive_vignette_intensity(
    time: Res<Time>,
    window: Query<&Window>,
    handle: Option<Res<VignetteMaterialHandle>>,
    mut materials: ResMut<Assets<RedAlertVignetteMaterial>>,
    mut flash: ResMut<ShieldFlashState>,
) {
    let Some(handle) = handle else { return };
    let Some(material) = materials.get_mut(&handle.0) else { return };

    // Decay flash intensity toward zero.
    flash.intensity = (flash.intensity - time.delta_secs() * FLASH_DECAY_RATE).max(0.0);
    material.flash_intensity = flash.intensity;

    // CSS owns the red-alert vignette now; keep the Bevy ring dark.
    material.intensity = 0.0;

    // Keep aspect ratio in sync with the window (handles resize).
    if let Some(window) = window.iter().next() {
        material.aspect_ratio = (window.width() / window.height()).max(0.01);
    }
}

// ── Pure helpers ─────────────────────────────────────────────────────

/// Convert a ship yaw in radians to a 0–359 integer compass bearing.
///
/// `yaw == 0` means the ship faces forward; the bearing increases
/// clockwise as viewed from above (equivalent to `360 − yaw°`).
/// Negative yaw and multi-turn yaw wrap correctly. The rounding
/// boundary at 359.5° rounds up to 360 then wraps back to 0 (never
/// returns 360).
pub fn yaw_to_compass_bearing(yaw_radians: f32) -> u32 {
    let degrees = (-yaw_radians).to_degrees().rem_euclid(360.0);
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

// ── HUD state push (issue #422) ──────────────────────────────────────
//
// The in-game HEADING/HULL/CONDITION readout is now rendered by the HTML
// viewscreen overlay. The Bevy side recomputes the serialised HUD state from
// `ShipState` + `ShipHullIntegrity` each frame, writes it into a single
// `ViewscreenHud` component only when it changes, and a `Changed<ViewscreenHud>`
// system encodes + emits a `HudStateChanged` message. The wasm forwarding to
// JS lives in `bridge::flush_hud_state`.

/// Single-entity component carrying the latest serialised HUD state. Bevy
/// change-detection drives the JS push.
#[derive(Component, Clone, PartialEq)]
struct ViewscreenHud(ViewscreenHudState);

/// Startup system: spawn the single entity that carries the HUD state.
fn spawn_hud_state_entity(mut commands: Commands) {
    commands.spawn(ViewscreenHud(ViewscreenHudState {
        heading: 0,
        hull_pct: 100,
        condition: "NOMINAL".to_string(),
        red_alert: false,
    }));
}

/// Compute the current HUD state from ship + hull resources. Reuses the exact
/// formulas from the retired in-game HUD strip.
fn compute_hud_state(ship: &ShipState, hull: &ShipHullIntegrity) -> ViewscreenHudState {
    let alert = ship.red_alert();
    let hull_pct = if hull.0.total_max() > 0.0 {
        (hull.0.total_current() / hull.0.total_max() * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    ViewscreenHudState {
        heading: yaw_to_compass_bearing(ship.yaw),
        hull_pct: hull_pct.round() as i32,
        condition: if alert { "ALERT" } else { "NOMINAL" }.to_string(),
        red_alert: alert,
    }
}

/// Per-frame system: recompute the HUD state and write it into the
/// `ViewscreenHud` component only when it differs, so `Changed<ViewscreenHud>`
/// fires only on actual change.
fn recompute_hud_state(
    ship: Option<Res<ShipState>>,
    hull: Option<Res<ShipHullIntegrity>>,
    mut hud_q: Query<&mut ViewscreenHud>,
) {
    let Some(ship) = ship else { return };
    let Some(hull) = hull else { return };
    let next = compute_hud_state(&ship, &hull);
    for mut hud in hud_q.iter_mut() {
        if hud.0 != next {
            hud.0 = next.clone();
        }
    }
}

/// `Changed<ViewscreenHud>` system: encode the HUD state and emit a
/// `HudStateChanged` message for the wasm bridge to forward to JS.
fn push_hud_state(
    hud_q: Query<&ViewscreenHud, Changed<ViewscreenHud>>,
    mut writer: MessageWriter<HudStateChanged>,
) {
    for hud in hud_q.iter() {
        if let Ok(json) = codec::encode_hud_state(&hud.0) {
            writer.write(HudStateChanged { json });
        }
    }
}

// ── Lobby screen systems ──────────────────────────────────────────────
// Removed in issue #436 — `spawn_lobby_screen`, `toggle_lobby_screen_visibility`,
// `rebuild_lobby_station_grid`, `update_lobby_header_values`, `spawn_station_card`,
// and `spawn_station_placeholder` were deleted. The lobby UI is now rendered
// entirely by the HTML overlay in `server.html` (`window.__updateLobby`),
// driven by `LobbyStatePayload` snapshots emitted from `push_lobby_state`.
#[cfg(test)]
mod tests {
    use super::*;

    use crate::damage::ConsoleHull;
    use crate::messages::Console;

    // ── compute_hud_state ────────────────────────────────────────────

    fn hull_at(current: f32, max: f32) -> ShipHullIntegrity {
        // ConsoleHull built from a single console so total_current/total_max
        // are exactly the values we want; apply damage to lower current.
        let mut hull = ConsoleHull::from_config(&[(Console::Helm, max)]);
        let mut rng = rand::rng();
        hull.apply_damage(max - current, &mut rng);
        ShipHullIntegrity(hull)
    }

    #[test]
    fn compute_hud_state_nominal() {
        let ship = ShipState::new();
        let hull = hull_at(100.0, 100.0);
        let state = compute_hud_state(&ship, &hull);
        assert_eq!(state.heading, 0);
        assert_eq!(state.hull_pct, 100);
        assert_eq!(state.condition, "NOMINAL");
        assert!(!state.red_alert);
    }

    #[test]
    fn compute_hud_state_alert_and_partial_hull() {
        let mut ship = ShipState::new();
        ship.toggle_red_alert();
        ship.yaw = std::f32::consts::FRAC_PI_2; // → bearing 270
        let hull = hull_at(50.0, 100.0);
        let state = compute_hud_state(&ship, &hull);
        assert_eq!(state.heading, 270);
        assert_eq!(state.hull_pct, 50);
        assert_eq!(state.condition, "ALERT");
        assert!(state.red_alert);
    }

    // ── yaw_to_compass_bearing ───────────────────────────────────────

    #[test]
    fn bearing_zero_yaw_is_zero() {
        assert_eq!(yaw_to_compass_bearing(0.0), 0);
    }

    #[test]
    fn bearing_quarter_turn_is_two_seventy() {
        assert_eq!(yaw_to_compass_bearing(std::f32::consts::FRAC_PI_2), 270);
    }

    #[test]
    fn bearing_half_turn_is_one_eighty() {
        assert_eq!(yaw_to_compass_bearing(std::f32::consts::PI), 180);
    }

    #[test]
    fn bearing_three_quarter_turn_is_ninety() {
        assert_eq!(yaw_to_compass_bearing(3.0 * std::f32::consts::FRAC_PI_2), 90);
    }

    #[test]
    fn bearing_full_turn_wraps_to_zero() {
        assert_eq!(yaw_to_compass_bearing(std::f32::consts::TAU), 0);
    }

    #[test]
    fn bearing_negative_yaw_wraps_positive() {
        // -π/2 rad = -90° → 90°
        assert_eq!(yaw_to_compass_bearing(-std::f32::consts::FRAC_PI_2), 90);
    }

    #[test]
    fn bearing_multi_turn_yaw_wraps() {
        // 2.5 turns: 2τ + π/2 → 270°
        let yaw = 2.0 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2;
        assert_eq!(yaw_to_compass_bearing(yaw), 270);
    }

    #[test]
    fn bearing_rounds_359_5_to_zero_not_360() {
        // -0.5° → 359.5°, rounds to 360 then wraps to 0.
        let yaw = 0.5_f32.to_radians();
        assert_eq!(yaw_to_compass_bearing(yaw), 0);
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
