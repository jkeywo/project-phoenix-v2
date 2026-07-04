//! Viewscreen combat feedback and HUD state push.
//!
//! The visible border frame, lobby UI, in-game HEADING/HULL/CONDITION
//! readout, and red-alert vignette are all rendered by the HTML overlay in
//! `server.html` (issues #422/#436); the corresponding Bevy UI paths were
//! removed. What remains here is everything the HTML overlay can't do:
//!
//! - **Shield-hit white flash** — [`RedAlertVignetteMaterial`] over the 3D
//!   scene, driven by [`process_shield_flash`] / [`drive_vignette_intensity`].
//! - **Hull-damage screen shake** — [`process_hull_shake`] /
//!   [`apply_camera_shake`] jitter the active `GameCamera` (native) or
//!   forward pixel offsets to JS for a CSS `transform: translate()` on the
//!   whole page (WASM).
//! - **HUD state push** — [`recompute_hud_state`] / [`push_hud_state`]
//!   serialise heading/hull/condition for the HTML overlay via
//!   `HudStateChanged`.
//! - **Lobby state push** — [`push_lobby_state`] serialises station/crew
//!   snapshots for the HTML lobby via `LobbyStateChanged`.
//!
//! Server-only — gated by the `server` feature in `lib.rs`.

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;
use bevy::ui_render::prelude::{UiMaterial, UiMaterialPlugin};

use rand::Rng;

use crate::codec;
use crate::console_bridge::{HudStateChanged, LobbyStateChanged};
use crate::lobby::{OutboundMessage, Sessions, WorldResource};
use crate::messages::{
    GamePhase, LobbyStatePayload, ServerMessage, StationPayload, ViewscreenHudState,
};
use crate::server::asset_preload::AssetPreloadResource;
use crate::server::renderer::GameCamera;
use crate::server_app::GameOverReason;
use crate::ship_state::ShipPhysics;
use crate::sim_sets::SimSet;
use crate::stations_config::ShipStations;

// ── Shield flash constants ────────────────────────────────────────────

/// Rate at which shield-hit flash decays per second (1.0 → 0.0 in 0.3 s).
const FLASH_DECAY_RATE: f32 = 1.0 / 0.3;

/// Rolling window for hull-damage screen shake accumulator (seconds).
/// Entries older than this are pruned each frame.
const SHAKE_WINDOW_SECS: f32 = 2.0;

/// Maximum shake magnitude in CSS pixels (WASM) or world units (native).
const SHAKE_MAX_MAGNITUDE: f32 = 2.5;

// ── Resources ────────────────────────────────────────────────────────

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

/// Tracks hull-damage screen shake on the viewscreen using a rolling
/// 2-second window of damage entries.
///
/// Each frame [`process_hull_shake`] pushes `(timestamp, hull_damage)`
/// entries; [`apply_camera_shake`] prunes entries outside the window,
/// sums the remaining damage, and derives a shake magnitude from the sum.
#[derive(Resource, Default)]
pub struct ShakeState {
    /// Rolling window of `(simulation_time, hull_damage)` entries.
    /// Pruned to the last [`SHAKE_WINDOW_SECS`] each frame.
    pub entries: Vec<(f32, f32)>,
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

/// Registers the shield-flash vignette material, hull-shake camera systems,
/// and the HUD/lobby state pushes for the HTML overlay.
pub struct ViewscreenBorderPlugin;

impl Plugin for ViewscreenBorderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(UiMaterialPlugin::<RedAlertVignetteMaterial>::default())
            .add_message::<HudStateChanged>()
            .add_message::<LobbyStateChanged>()
            .init_resource::<ShieldFlashState>()
            .init_resource::<ShakeState>()
            // The border frame, lobby UI, and red-alert vignette are rendered by the
            // HTML overlay in `server.html` (`window.__updateLobby` / `__updateHud`);
            // this plugin pushes the `LobbyStatePayload` / `ViewscreenHudState`
            // snapshots that drive it. The Bevy border/lobby UI trees were deleted in
            // issues #422/#436 — see wiki/concepts/server-lobby-ui.md.
            .add_systems(
                Startup,
                (setup_vignette_material, spawn_hud_state_entity).chain(),
            )
            .add_systems(
                Update,
                (
                    process_shield_flash.after(SimSet::Broadcast),
                    drive_vignette_intensity.after(process_shield_flash),
                    process_hull_shake.after(SimSet::Broadcast),
                    apply_camera_shake
                        .after(process_shield_flash)
                        .after(process_hull_shake)
                        .run_if(in_state(GamePhase::InProgress)),
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
            )
            // On GameOver, push one final HUD state with the game-over message.
            .add_systems(OnEnter(GamePhase::GameOver), push_game_over_hud_state);
    }
}

/// Reads server lobby resources and emits `LobbyStateChanged` for the HTML
/// lobby overlay whenever the state changes. Runs in `Update` so the bridge's
/// `flush_lobby_state` (in `PostUpdate`) forwards it to JS.
pub(crate) fn push_lobby_state(
    sessions: Option<Res<Sessions>>,
    ship_stations: Option<Res<ShipStations>>,
    phase: Res<State<GamePhase>>,
    world_resource: Option<Res<WorldResource>>,
    preload: Option<Res<AssetPreloadResource>>,
    mut writer: MessageWriter<LobbyStateChanged>,
) {
    let Some(sessions) = sessions else { return };
    let Some(stations) = ship_stations else {
        return;
    };

    let players = sessions.0.players();
    let connected_count = players.iter().filter(|p| p.connected).count() as u32;
    let roster_size = stations.stations.len() as u32;

    let mut station_payloads: Vec<StationPayload> = Vec::new();
    let mut spectators: Vec<String> = Vec::new();

    for def in &stations.stations {
        let holder = players
            .iter()
            .find(|p| p.connected && p.station.as_ref() == Some(&def.id));
        station_payloads.push(StationPayload {
            name: def.name.clone(),
            short_code: def.short_code.clone(),
            rank: def.rank.clone(),
            holder_name: holder.map(|p| p.name.clone()),
            is_mine: false,
            preset_names: vec![],
        });
    }

    // Players with no station who are connected are spectators.
    if roster_size > 0 && connected_count > roster_size {
        for p in players
            .iter()
            .filter(|p| p.connected && p.station.is_none())
        {
            spectators.push(p.name.clone());
        }
    }

    let all_filled =
        !stations.stations.is_empty() && station_payloads.iter().all(|s| s.holder_name.is_some());

    let scenario_title = world_resource
        .as_ref()
        .map(|w| w.0.scenario_title.clone())
        .unwrap_or_default();

    let scenario_body = world_resource
        .as_ref()
        .map(|w| w.0.scenario_description.clone())
        .unwrap_or_default();

    let all_ready = sessions.0.all_ready();

    let loading_progress = if *phase.get() == GamePhase::Loading {
        preload
            .as_ref()
            .filter(|p| p.started)
            .map(|p| p.fraction())
    } else {
        None
    };

    let payload = LobbyStatePayload {
        phase: format!("{:?}", phase.get()),
        scenario_title,
        scenario_body,
        crew_count: station_payloads
            .iter()
            .filter(|s| s.holder_name.is_some())
            .count() as u32,
        max_players: roster_size,
        all_stations_filled: all_filled,
        all_ready,
        stations: station_payloads,
        spectators,
        loading_progress,
    };

    if let Ok(json) = codec::encode_lobby_state(&payload) {
        writer.write(LobbyStateChanged { json });
    }
}

// ── Systems ──────────────────────────────────────────────────────────

/// Creates the single shield-flash vignette material and caches its handle.
/// (The red-alert vignette and border frame are HTML-owned; only the
/// shield-hit white flash still renders through Bevy.)
fn setup_vignette_material(
    mut commands: Commands,
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
    commands.insert_resource(VignetteMaterialHandle(vignette));
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
/// `hull > 0` and pushes `(timestamp, hull)` entries into the rolling
/// window [`ShakeState`].
///
/// Runs after `SimSet::Broadcast` so the outbox has been drained into
/// `OutboundMessage` messages and is safe to read.
fn process_hull_shake(
    mut outbound: MessageReader<OutboundMessage>,
    mut shake: ResMut<ShakeState>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs();
    for msg in outbound.read() {
        if let ServerMessage::DamageTaken { hull, .. } = &msg.msg {
            if *hull > 0.0 {
                shake.entries.push((now, *hull));
            }
        }
    }
}

/// Computes a screen-space offset from the rolling 2-second damage window
/// in [`ShakeState`], applies it, and prunes expired entries.
///
/// On native (non-WASM) the offset is applied to the 3D camera transform,
/// shaking the Bevy viewport. On WASM the offset is forwarded to JavaScript
/// which applies a CSS `transform: translate()` to the whole page, so the
/// canvas *and* HTML overlay elements (border, HUD) shake together.
///
/// Runs after [`process_hull_shake`] (so new damage entries are already
/// pushed) and after [`hull_camera`] in the renderer plugin (so the base
/// camera position is already set).
///
/// When no damage has been taken recently the offset is reset to zero.
fn apply_camera_shake(
    time: Res<Time>,
    mut shake: ResMut<ShakeState>,
    #[cfg(not(target_arch = "wasm32"))] mut cam_query: Query<&mut Transform, With<GameCamera>>,
) {
    let now = time.elapsed_secs();

    // Prune entries outside the rolling window.
    shake.entries.retain(|&(t, _)| now - t <= SHAKE_WINDOW_SECS);

    // Sum hull damage in the window and derive magnitude.
    let total_hull: f32 = shake.entries.iter().map(|&(_, h)| h).sum();
    let magnitude = (total_hull / 30.0).min(1.0) * SHAKE_MAX_MAGNITUDE;

    if magnitude > 0.01 {
        let mut rng = rand::rng();
        let offset_x = rng.random_range(-magnitude..magnitude);
        let offset_y = rng.random_range(-magnitude..magnitude);

        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Ok(mut transform) = cam_query.single_mut() {
                transform.translation.x += offset_x;
                transform.translation.y += offset_y;
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            crate::server::bridge::set_shake_offset(offset_x, offset_y);
        }
    } else {
        #[cfg(target_arch = "wasm32")]
        {
            crate::server::bridge::set_shake_offset(0.0, 0.0);
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
    let Some(material) = materials.get_mut(&handle.0) else {
        return;
    };

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
/// `yaw == 0` means the ship faces North (−Z); positive yaw is a
/// clockwise (starboard) turn — a quarter-turn right gives 090° (East).
/// Negative yaw and multi-turn yaw wrap correctly. The rounding
/// boundary at 359.5° rounds up to 360 then wraps back to 0 (never
/// returns 360).
pub fn yaw_to_compass_bearing(yaw_radians: f32) -> u32 {
    let degrees = yaw_radians.to_degrees().rem_euclid(360.0);
    (degrees.round() as u32) % 360
}

// ── HUD state push (issue #422) ──────────────────────────────────────
//
// The in-game HEADING/HULL/CONDITION readout is now rendered by the HTML
// viewscreen overlay. The Bevy side recomputes the serialised HUD state from
// the LocalShip's `ShipPhysics` + `EntitySystemHull` components each frame,
// writes it into a single `ViewscreenHud` component only when it changes, and
// a `Changed<ViewscreenHud>` system encodes + emits a `HudStateChanged`
// message. The wasm forwarding to JS lives in `bridge::flush_hud_state`.

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
        engine_thrust: 0.0,
        game_over_message: None,
    }));
}

/// Compute the current HUD state from ship + hull resources. Reuses the exact
/// formulas from the retired in-game HUD strip.
fn compute_hud_state(
    red_alert: bool,
    physics: &ShipPhysics,
    hull_current: f32,
    hull_max: f32,
    engine_thrust: f32,
    phase: &GamePhase,
    game_over_reason: Option<&GameOverReason>,
) -> ViewscreenHudState {
    let alert = red_alert;
    let hull_pct = if hull_max > 0.0 {
        (hull_current / hull_max * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    let game_over_message = if *phase == GamePhase::GameOver {
        let reason = game_over_reason.and_then(|r| r.0.as_deref()).unwrap_or("");
        let msg = if reason.starts_with("All consoles destroyed")
            || reason.starts_with("Ship destroyed")
        {
            "Ship Destroyed".to_string()
        } else {
            reason.to_string()
        };
        Some(msg)
    } else {
        None
    };
    ViewscreenHudState {
        heading: yaw_to_compass_bearing(physics.yaw),
        hull_pct: hull_pct.round() as i32,
        condition: if alert { "ALERT" } else { "NOMINAL" }.to_string(),
        red_alert: alert,
        engine_thrust,
        game_over_message,
    }
}

/// Per-frame system: recompute the HUD state and write it into the
/// `ViewscreenHud` component only when it differs, so `Changed<ViewscreenHud>`
/// fires only on actual change.
fn recompute_hud_state(
    red_alert_q: Query<&crate::ship_state::ShipRedAlert, With<crate::simulation::LocalShip>>,
    hull_q: Query<&crate::entity_spawner::EntitySystemHull, With<crate::simulation::LocalShip>>,
    phase: Option<Res<State<GamePhase>>>,
    game_over_reason: Option<Res<GameOverReason>>,
    physics_q: Query<&ShipPhysics, With<crate::simulation::LocalShip>>,
    last_input_q: Query<&crate::ship_plugin::LastHelmInput, With<crate::simulation::LocalShip>>,
    mut hud_q: Query<&mut ViewscreenHud>,
) {
    let Some(phase) = phase else { return };
    let physics = physics_q.single().ok().copied().unwrap_or_default();
    let red_alert = red_alert_q.single().map(|ra| ra.0).unwrap_or(false);
    let (hull_current, hull_max) = hull_q
        .single()
        .map(|h| (h.0.total_current(), h.0.total_max()))
        .unwrap_or((100.0, 100.0));
    let engine_thrust = last_input_q
        .iter()
        .next()
        .map(|li| li.thrust.abs())
        .unwrap_or(0.0);
    let next = compute_hud_state(
        red_alert,
        &physics,
        hull_current,
        hull_max,
        engine_thrust,
        phase.get(),
        game_over_reason.as_deref(),
    );
    for mut hud in hud_q.iter_mut() {
        if hud.0 != next {
            hud.0 = next.clone();
        }
    }
}

/// `OnEnter(GamePhase::GameOver)` system: push one final HUD state.
fn push_game_over_hud_state(
    red_alert_q: Query<&crate::ship_state::ShipRedAlert, With<crate::simulation::LocalShip>>,
    hull_q: Query<&crate::entity_spawner::EntitySystemHull, With<crate::simulation::LocalShip>>,
    game_over_reason: Option<Res<GameOverReason>>,
    physics_q: Query<&ShipPhysics, With<crate::simulation::LocalShip>>,
    mut hud_q: Query<&mut ViewscreenHud>,
    mut writer: MessageWriter<HudStateChanged>,
) {
    let physics = physics_q.single().ok().copied().unwrap_or_default();
    let red_alert = red_alert_q.single().map(|ra| ra.0).unwrap_or(false);
    let (hull_current, hull_max) = hull_q
        .single()
        .map(|h| (h.0.total_current(), h.0.total_max()))
        .unwrap_or((100.0, 100.0));
    let next = compute_hud_state(
        red_alert,
        &physics,
        hull_current,
        hull_max,
        0.0,
        &GamePhase::GameOver,
        game_over_reason.as_deref(),
    );
    for mut hud in hud_q.iter_mut() {
        hud.0 = next.clone();
    }
    if let Ok(json) = codec::encode_hud_state(&next) {
        writer.write(HudStateChanged { json });
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

    // ── compute_hud_state ────────────────────────────────────────────

    #[test]
    fn compute_hud_state_nominal() {
        let physics = ShipPhysics::default();
        let state = compute_hud_state(false, &physics, 100.0, 100.0, 0.0, &GamePhase::InProgress, None);
        assert_eq!(state.heading, 0);
        assert_eq!(state.hull_pct, 100);
        assert_eq!(state.condition, "NOMINAL");
        assert!(!state.red_alert);
        assert_eq!(state.engine_thrust, 0.0);
        assert!(state.game_over_message.is_none());
    }

    #[test]
    fn compute_hud_state_alert_and_partial_hull() {
        let physics = ShipPhysics {
            yaw: std::f32::consts::FRAC_PI_2,
            ..Default::default()
        };
        let state = compute_hud_state(true, &physics, 50.0, 100.0, 0.75, &GamePhase::InProgress, None);
        assert_eq!(state.heading, 90);
        assert_eq!(state.hull_pct, 50);
        assert_eq!(state.condition, "ALERT");
        assert!(state.red_alert);
        assert!((state.engine_thrust - 0.75).abs() < f32::EPSILON);
    }

    #[test]
    fn compute_hud_state_engine_thrust_propagated() {
        let physics = ShipPhysics::default();
        let state = compute_hud_state(false, &physics, 100.0, 100.0, 0.5, &GamePhase::InProgress, None);
        assert_eq!(state.engine_thrust, 0.5);
    }

    #[test]
    fn compute_hud_state_game_over_ship_destroyed() {
        use crate::server_app::GameOverReason;
        let physics = ShipPhysics::default();
        let reason = GameOverReason(Some("All consoles destroyed".into()));
        let state = compute_hud_state(
            false,
            &physics,
            0.0,
            100.0,
            0.0,
            &GamePhase::GameOver,
            Some(&reason),
        );
        assert_eq!(state.game_over_message.as_deref(), Some("Ship Destroyed"));
    }

    #[test]
    fn compute_hud_state_game_over_scenario_message() {
        use crate::server_app::GameOverReason;
        let physics = ShipPhysics::default();
        let reason = GameOverReason(Some("VICTORY: All enemies eliminated.".into()));
        let state = compute_hud_state(
            false,
            &physics,
            50.0,
            100.0,
            0.0,
            &GamePhase::GameOver,
            Some(&reason),
        );
        assert_eq!(
            state.game_over_message.as_deref(),
            Some("VICTORY: All enemies eliminated.")
        );
    }

    // ── yaw_to_compass_bearing ───────────────────────────────────────

    #[test]
    fn bearing_zero_yaw_is_zero() {
        assert_eq!(yaw_to_compass_bearing(0.0), 0);
    }

    #[test]
    fn bearing_quarter_turn_is_ninety() {
        // +π/2 = right turn (clockwise) → ship faces East → 090°
        assert_eq!(yaw_to_compass_bearing(std::f32::consts::FRAC_PI_2), 90);
    }

    #[test]
    fn bearing_half_turn_is_one_eighty() {
        assert_eq!(yaw_to_compass_bearing(std::f32::consts::PI), 180);
    }

    #[test]
    fn bearing_three_quarter_turn_is_two_seventy() {
        // 3*π/2 = three-quarter clockwise turn → ship faces West → 270°
        assert_eq!(
            yaw_to_compass_bearing(3.0 * std::f32::consts::FRAC_PI_2),
            270
        );
    }

    #[test]
    fn bearing_full_turn_wraps_to_zero() {
        assert_eq!(yaw_to_compass_bearing(std::f32::consts::TAU), 0);
    }

    #[test]
    fn bearing_negative_yaw_wraps_positive() {
        // -π/2 = left turn (counter-clockwise) → ship faces West → 270°
        assert_eq!(yaw_to_compass_bearing(-std::f32::consts::FRAC_PI_2), 270);
    }

    #[test]
    fn bearing_multi_turn_yaw_wraps() {
        // 2.5 turns clockwise: 2τ + π/2 → same as π/2 → 090°
        let yaw = 2.0 * std::f32::consts::TAU + std::f32::consts::FRAC_PI_2;
        assert_eq!(yaw_to_compass_bearing(yaw), 90);
    }

    #[test]
    fn bearing_rounds_359_5_to_zero_not_360() {
        // -0.5° (tiny left turn) → 359.5°, rounds to 360 then wraps to 0.
        let yaw = (-0.5_f32).to_radians();
        assert_eq!(yaw_to_compass_bearing(yaw), 0);
    }
}
