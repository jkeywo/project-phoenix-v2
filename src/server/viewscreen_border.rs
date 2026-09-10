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

use crate::console_bridge::{HudStateChanged, LobbyStateChanged};
use crate::core::codec;
use crate::core::computer_message::ActiveComputerMessage;
use crate::core::messages::{
    ComputerMessageWire, GamePhase, LobbyStatePayload, Player, ServerMessage, StationPayload,
    ViewscreenHudState,
};
use crate::lobby::stations_config::ShipStations;
use crate::lobby::{CountdownTimer, OutboundMessage, Sessions, WorldResource};
#[cfg(not(target_arch = "wasm32"))]
use crate::render_setup::GameCamera;
use crate::server::asset_preload::AssetPreloadResource;
use crate::server_app::GameOverReason;
use crate::ship::state::ShipPhysics;

// ── Shield flash constants ────────────────────────────────────────────

/// Rate at which shield-hit flash decays per second (1.0 → 0.0 in 0.3 s).
const FLASH_DECAY_RATE: f32 = 1.0 / 0.3;

/// Rolling window for hull-damage screen shake accumulator (seconds).
/// Entries older than this are pruned each frame.
const SHAKE_WINDOW_SECS: f32 = 2.0;

/// Maximum shake magnitude in CSS pixels (WASM) or world units (native).
const SHAKE_MAX_MAGNITUDE: f32 = 2.5;

/// Total hull damage in the rolling window that saturates the shake to
/// [`SHAKE_MAX_MAGNITUDE`]. It doubles as the shield-absorb amount that
/// saturates the white flash — the two feedbacks share one "full hit" scale.
const SHAKE_DAMAGE_FULL: f32 = 30.0;

/// Default global shake-intensity scale (issue #1173, PRD #1168). The
/// comfort-slider seam: [`ViewscreenMotion::shake_intensity`] multiplies the
/// derived magnitude, so a future accessibility "screen shake" slider drives
/// `0.0..=1.0` through this one field. `1.0` is the shipped, no-reduction
/// baseline — normal motion is unchanged when nothing requests a reduction.
const DEFAULT_SHAKE_INTENSITY: f32 = 1.0;

/// The shipped flash intensity when nothing has asked for less (issue #1428).
const DEFAULT_FLASH_INTENSITY: f32 = 1.0;

/// The shipped decorative-motion intensity when nothing has asked for less
/// (issue #1428).
const DEFAULT_DECORATIVE_INTENSITY: f32 = 1.0;

/// The intensity an effect that FOLLOWS the motion preference takes when that
/// preference asks for reduction. `0.0` — the same answer `shake_magnitude`
/// and the old `REDUCED_MOTION_FLASH_CAP` gave under reduce in issue #1173, so
/// an operator who never opens the new controls sees no change at all.
const REDUCED_MOTION_INTENSITY: f32 = 0.0;

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

/// Host-side viewscreen motion-comfort state (issue #1173, PRD #1168).
///
/// PRESENTATION state — never folded into the sim digest; classified
/// `presentation` in `tests/authoritative_state_enumeration.rs`. It carries the
/// host's reduced-motion preference to the render systems, and is set on both
/// render paths:
///
/// - **WASM**: the host page forwards `prefers-reduced-motion: reduce` through
///   [`crate::server::bridge::wasm_set_reduced_motion`], which
///   [`sync_reduced_motion`] drains into this resource each frame.
/// - **Native**: [`init_native_reduced_motion`] seeds it once at startup from
///   the `PHOENIX_REDUCED_MOTION` environment variable — the desktop viewscreen
///   has no DOM `prefers-reduced-motion` to read.
///
/// Both `#[cfg]` branches of [`apply_camera_shake`] and
/// [`drive_vignette_intensity`] read it, so the reduced-motion decision reaches
/// the native camera jitter and the WASM whole-page translate identically.
#[derive(Resource, Debug, Clone)]
pub struct ViewscreenMotion {
    /// The reduced-motion preference this endpoint reported.
    ///
    /// Since issue #1428 it is no longer what zeroes the shake and the flash —
    /// the two intensities below are, and they are what the settings surfaces
    /// write. What this still does is supply the DEFAULT those intensities take
    /// when nothing has published one: `0.0` under reduce, `1.0` otherwise,
    /// which is exactly the behaviour issue #1173 shipped. So a build whose
    /// page only ever calls [`crate::server::bridge::wasm_set_reduced_motion`]
    /// behaves as it always did, and a page that publishes intensities
    /// overrides it in both directions — including keeping the shake on a
    /// machine whose OS asked to reduce motion.
    pub reduced_motion: bool,
    /// Camera/page shake intensity in `0.0..=1.0`. Multiplies the derived
    /// magnitude in [`shake_magnitude`]; `0.0` is off. Written by
    /// [`sync_viewscreen_motion`] from whatever the endpoint published.
    pub shake_intensity: f32,
    /// Shield-hit white-flash intensity in `0.0..=1.0`, the separate lever
    /// PRD #1418 story 13 asks for. Multiplies the decayed flash in
    /// [`scaled_flash_intensity`]; `0.0` is off.
    pub flash_intensity: f32,
    /// Decorative interface-motion intensity in `0.0..=1.0` — the third lever
    /// PRD #1418 story 13 asks for, and the only one of the three no renderer
    /// reads (issue #1428).
    ///
    /// It is resolved here anyway, beside the two that are, because the surface
    /// that needs it is a DOCUMENT the host has to be told about: the native
    /// Viewscreen draws its frame and its red-alert vignette in
    /// `gui/viewscreen-hud.html`, which no head injection reaches, so
    /// `native_host::panes::ultralight::cache_hud_state` reads this field and
    /// pushes all three bands to that document. On the web the page stamps its
    /// own root from the endpoint record and nothing reads this.
    pub decorative_intensity: f32,
}

impl Default for ViewscreenMotion {
    fn default() -> Self {
        Self {
            reduced_motion: false,
            shake_intensity: DEFAULT_SHAKE_INTENSITY,
            flash_intensity: DEFAULT_FLASH_INTENSITY,
            decorative_intensity: DEFAULT_DECORATIVE_INTENSITY,
        }
    }
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
        // Authoritative-state exclusion declaration (issue #1221, Track 3 step C9).
        // `ViewscreenMotion` is PRESENTATION — the reduced-motion comfort profile
        // and shake-intensity scale that only decide how the hull-damage shake and
        // shield flash are DRAWN; nothing in the fixed tick reads it. Declared here
        // at its owning site, replacing the `EXCLUSIONS` const in
        // `tests/authoritative_state_enumeration.rs`. This server-only plugin is
        // absent from a headless run, so the declaration never reaches that census
        // — correct, because `ViewscreenMotion` never registers there either. Inert
        // to the digest.
        {
            use crate::authoritative::{DeclareState, StateClass};
            app.declare_state::<ViewscreenMotion>(
                StateClass::Presentation,
                "viewscreen-motion-state",
            );
        }
        app.add_plugins(UiMaterialPlugin::<RedAlertVignetteMaterial>::default())
            .add_message::<HudStateChanged>()
            .add_message::<LobbyStateChanged>()
            .init_resource::<ShieldFlashState>()
            .init_resource::<ShakeState>()
            .init_resource::<ViewscreenMotion>()
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
                    // No `.after(SimSet::Broadcast)` edges since issue #895:
                    // the sim (and its Broadcast set) runs in `FixedUpdate`,
                    // which always completes before `Update` in a frame, so
                    // the outbox has already been drained into
                    // `OutboundMessage` when these frame-side readers run.
                    process_shield_flash,
                    drive_vignette_intensity.after(process_shield_flash),
                    process_hull_shake,
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
            // Capture the final HUD before the terminal broadcast consumes the
            // per-ending reason. Both native and browser hosts use this edge.
            .add_systems(
                OnEnter(GamePhase::GameOver),
                push_game_over_hud_state.before(crate::server_app::on_game_over_enter),
            );

        // Reduced-motion source, one per render path (issue #1173, AC3). The
        // WASM host forwards `prefers-reduced-motion` live; the native build
        // seeds the preference once at startup from the environment. Both write
        // the same `ViewscreenMotion` resource the shake/flash systems read.
        app.add_systems(
            Update,
            sync_viewscreen_motion
                .before(apply_camera_shake)
                .before(drive_vignette_intensity),
        );
        #[cfg(not(target_arch = "wasm32"))]
        app.add_systems(Startup, init_native_reduced_motion);
    }
}

/// Drain what the endpoint has published about motion into [`ViewscreenMotion`]
/// every frame (issues #1173 and #1428).
///
/// Three values arrive from the same seam and are resolved by one rule:
///
///  * the reduced-motion preference — on WASM the host page's
///    `prefers-reduced-motion: reduce` through
///    [`crate::server::bridge::wasm_set_reduced_motion`]; on native the value
///    [`init_native_reduced_motion`] seeded once from the environment;
///  * a published shake intensity, a published flash intensity and a published
///    decorative-motion intensity, each `Option` because *nothing published* and
///    *published as zero* are different facts. The first takes the preference's
///    default; the second is an operator who asked for silence and must not be
///    overridden by anything.
///
/// The third is resolved here rather than where it is consumed so that one rule
/// answers all three: the native HUD-overlay document's bands would otherwise
/// disagree with the shader beside them on a display following the machine.
///
/// Read every frame rather than once at init so a change — an OS preference
/// flipped, a Display-tab press — takes effect without a reload. Guarded
/// assignment, so a frame that changes nothing does not wake Bevy's change
/// detection for three resources' worth of readers.
fn sync_viewscreen_motion(mut motion: ResMut<ViewscreenMotion>) {
    #[cfg(target_arch = "wasm32")]
    let reduced = crate::server::bridge::reduced_motion_requested();
    // Native has no live preference query: `init_native_reduced_motion` read the
    // environment seam once at startup, and that is what the resource holds.
    #[cfg(not(target_arch = "wasm32"))]
    let reduced = motion.reduced_motion;

    let (shake, flash, decorative) = crate::server::bridge::published_effect_intensities();
    let following = if reduced {
        REDUCED_MOTION_INTENSITY
    } else {
        DEFAULT_SHAKE_INTENSITY
    };
    let next_shake = shake.unwrap_or(following).clamp(0.0, 1.0);
    let next_flash = flash.unwrap_or(following).clamp(0.0, 1.0);
    let next_decorative = decorative.unwrap_or(following).clamp(0.0, 1.0);

    if motion.reduced_motion != reduced {
        motion.reduced_motion = reduced;
    }
    if motion.shake_intensity != next_shake {
        motion.shake_intensity = next_shake;
    }
    if motion.flash_intensity != next_flash {
        motion.flash_intensity = next_flash;
    }
    if motion.decorative_intensity != next_decorative {
        motion.decorative_intensity = next_decorative;
    }
}

/// Native: seed [`ViewscreenMotion::reduced_motion`] from the host environment
/// at startup (issue #1173). The desktop viewscreen has no DOM
/// `prefers-reduced-motion` to read, so
/// [`crate::server::bridge::native_reduced_motion`] reads the env-var seam that
/// is the native analog of the WASM host page forwarding the browser
/// preference. An unset/other value leaves the shipped default (normal motion).
#[cfg(not(target_arch = "wasm32"))]
fn init_native_reduced_motion(mut motion: ResMut<ViewscreenMotion>) {
    motion.reduced_motion = crate::server::bridge::native_reduced_motion();
}

/// Reads server lobby resources and emits `LobbyStateChanged` for the HTML
/// lobby overlay whenever the state changes. Runs in `Update` so the bridge's
/// `flush_host_channels` (in `PostUpdate`) forwards it to JS.
pub(crate) fn push_lobby_state(
    sessions: Option<Res<Sessions>>,
    ship_stations: Option<Res<ShipStations>>,
    phase: Res<State<GamePhase>>,
    world_resource: Option<Res<WorldResource>>,
    preload: Option<Res<AssetPreloadResource>>,
    model_rigs: Option<Res<crate::entities::model_markers::ModelRigReadiness>>,
    countdown: Option<Res<CountdownTimer>>,
    gm_roster: Option<Res<crate::gm_roster::GmRoster>>,
    mut writer: MessageWriter<LobbyStateChanged>,
) {
    let Some(sessions) = sessions else { return };
    let Some(stations) = ship_stations else {
        return;
    };

    let players = sessions.0.players();
    let roster = claimable_lobby_roster(&stations, players);
    let mut spectators: Vec<String> = Vec::new();

    // Spectators (issue #1105) are the participants who hold the explicit
    // Spectator role — a real session role now, not the connected-and-stationless
    // overflow heuristic this used to infer. The invariant (spectator ⇒ no
    // station) keeps this list free of anyone still holding a seat.
    for p in players.iter().filter(|p| p.connected && p.spectator) {
        spectators.push(p.name.clone());
    }

    let scenario_title = world_resource
        .as_ref()
        .map(|w| w.0.scenario_title.clone())
        .unwrap_or_default();

    let scenario_body = world_resource
        .as_ref()
        .map(|w| w.0.scenario_description.clone())
        .unwrap_or_default();

    let readiness = sessions.0.readiness_tally();
    let all_ready = readiness.all_ready();
    // The wire field predates authoritative rig preloading and retains its
    // protocol name. It now means the complete local start-assets gate: GPU
    // presentation when present, plus primary model rigs on every profile.
    let presentation_ready = local_start_assets_ready(preload.as_deref(), model_rigs.as_deref());

    let loading_progress = if *phase.get() == GamePhase::Loading {
        preload.as_ref().filter(|p| p.started).map(|p| p.fraction())
    } else {
        None
    };

    let countdown_secs = countdown
        .map(|t| t.remaining_secs.ceil() as u32)
        .unwrap_or(0);

    let payload = LobbyStatePayload {
        phase: format!("{:?}", phase.get()),
        scenario_title,
        scenario_body,
        crew_count: roster.crew_count,
        max_players: roster.max_players,
        all_stations_filled: roster.all_filled,
        all_ready,
        readiness,
        station_ratings: sessions.0.lobby_station_ratings(&stations),
        presentation_ready,
        stations: roster.stations,
        spectators,
        gms: gm_roster
            .as_ref()
            .map(|roster| roster.projection())
            .unwrap_or_default(),
        loading_progress,
        countdown_secs,
    };

    if let Ok(json) = codec::encode_lobby_state(&payload) {
        writer.write(LobbyStateChanged { json });
    }
}

/// Project the host-local presentation gate into the fleet lobby snapshot.
///
/// A present preload resource is ready only after its terminal `complete`
/// state. `AssetPreloadResource` counts failed render assets as terminal, so a
/// missing model cannot deadlock the fleet. A rendererless/headless app has no
/// preload resource and therefore no local presentation work to wait for.
fn local_presentation_ready(preload: Option<&AssetPreloadResource>) -> bool {
    match preload {
        Some(preload) => preload.complete,
        None => true,
    }
}

fn local_start_assets_ready(
    preload: Option<&AssetPreloadResource>,
    model_rigs: Option<&crate::entities::model_markers::ModelRigReadiness>,
) -> bool {
    local_presentation_ready(preload) && model_rigs.is_none_or(|rigs| rigs.is_ready())
}

/// Build the host viewscreen lobby roster from claimable bridge seats only.
/// Auxiliary Stations remain mounted for visiting placement, but they are not
/// seats and therefore must not affect cards or any counts derived from them.
struct ClaimableLobbyRoster {
    stations: Vec<StationPayload>,
    crew_count: u32,
    max_players: u32,
    all_filled: bool,
}

fn claimable_lobby_roster(stations: &ShipStations, players: &[Player]) -> ClaimableLobbyRoster {
    let station_payloads: Vec<_> = stations
        .stations
        .iter()
        .filter(|def| !def.auxiliary)
        .map(|def| {
            let holder = players
                .iter()
                .find(|p| p.connected && p.station.as_ref() == Some(&def.id));
            StationPayload {
                // The authoring key, carried so a native host can match this
                // card to its per-station screen row (issue #1331): the bridge
                // layout law is keyed on the same id.
                id: def.id.0.clone(),
                name: def.name.clone(),
                short_code: def.short_code.clone(),
                rank: def.rank.clone(),
                holder_name: holder.map(|p| p.name.clone()),
                is_mine: false,
                preset_names: vec![],
            }
        })
        .collect();
    let crew_count = station_payloads
        .iter()
        .filter(|station| station.holder_name.is_some())
        .count() as u32;
    ClaimableLobbyRoster {
        max_players: station_payloads.len() as u32,
        all_filled: !station_payloads.is_empty() && crew_count == station_payloads.len() as u32,
        stations: station_payloads,
        crew_count,
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
/// Runs in `Update`, after the fixed loop (where `SimSet::Broadcast` lives
/// since #895) has drained the outbox into `OutboundMessage` messages.
fn process_shield_flash(
    mut outbound: MessageReader<OutboundMessage>,
    mut flash: ResMut<ShieldFlashState>,
) {
    for msg in outbound.read() {
        if let ServerMessage::DamageTaken { shield, .. } = &msg.msg {
            if *shield > 0.0 {
                flash.intensity = (*shield / SHAKE_DAMAGE_FULL).min(1.0);
            }
        }
    }
}

/// Reads [`OutboundMessage`] for [`ServerMessage::DamageTaken`] with
/// `hull > 0` and pushes `(timestamp, hull)` entries into the rolling
/// window [`ShakeState`].
///
/// Runs in `Update`, after the fixed loop (where `SimSet::Broadcast` lives
/// since #895) has drained the outbox into `OutboundMessage` messages.
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
///
/// Issue #903: draws OS entropy via `rand::rng()` for the shake jitter, which
/// is why the fn carries the `disallowed_methods` allow — purely cosmetic
/// (screen-space camera offset), never read back into simulation state.
#[allow(clippy::disallowed_methods)]
fn apply_camera_shake(
    time: Res<Time>,
    motion: Res<ViewscreenMotion>,
    mut shake: ResMut<ShakeState>,
    #[cfg(not(target_arch = "wasm32"))] mut cam_query: Query<&mut Transform, With<GameCamera>>,
) {
    let now = time.elapsed_secs();

    // Prune entries outside the rolling window.
    shake.entries.retain(|&(t, _)| now - t <= SHAKE_WINDOW_SECS);

    // Sum hull damage in the window and derive magnitude. The whole
    // motion-comfort decision is already inside `motion.shake_intensity`
    // (issue #1428): an intensity of `0.0` — whether the operator turned shake
    // off outright or is following a preference that asks for reduction —
    // returns exactly `0.0` here, so BOTH the native camera-jitter branch and
    // the WASM whole-page-translate branch below collapse to the no-shake path.
    // Neither branch has a comfort rule of its own to keep in step.
    let total_hull: f32 = shake.entries.iter().map(|&(_, h)| h).sum();
    let magnitude = shake_magnitude(total_hull, motion.shake_intensity);

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
    motion: Res<ViewscreenMotion>,
    window: Query<&Window>,
    handle: Option<Res<VignetteMaterialHandle>>,
    mut materials: ResMut<Assets<RedAlertVignetteMaterial>>,
    mut flash: ResMut<ShieldFlashState>,
) {
    let Some(handle) = handle else { return };
    let Some(material) = materials.get_mut(&handle.0) else {
        return;
    };

    // Decay flash intensity toward zero, then scale it by the endpoint's flash
    // setting (issue #1428): a sudden full-frame white jolt is the effect with a
    // photosensitivity cost, so it has a lever of its own rather than riding the
    // motion preference. `1.0` passes the decayed value through unchanged.
    flash.intensity = (flash.intensity - time.delta_secs() * FLASH_DECAY_RATE).max(0.0);
    material.flash_intensity = scaled_flash_intensity(flash.intensity, motion.flash_intensity);

    // CSS owns the red-alert vignette now; keep the Bevy ring dark.
    material.intensity = 0.0;

    // Keep aspect ratio in sync with the window (handles resize).
    if let Some(window) = window.iter().next() {
        material.aspect_ratio = (window.width() / window.height()).max(0.01);
    }
}

// ── Pure helpers ─────────────────────────────────────────────────────

/// Derive the per-frame hull-shake magnitude — CSS pixels on WASM, world units
/// on native — from the rolling window's total hull damage (issue #1173).
///
/// `intensity` is [`ViewscreenMotion::shake_intensity`], the resolved comfort
/// scale in `0.0..=1.0` — `0.0` is off and returns exactly `0.0` regardless of
/// damage. Issue #1428 removed the separate `reduced` flag this used to take:
/// the reduced-motion preference is now folded into the intensity by
/// [`sync_viewscreen_motion`], so there is ONE number deciding how far the
/// picture moves rather than a flag and a scale that could disagree — and an
/// operator who explicitly keeps the shake on a reduce-motion machine gets it.
///
/// Both render paths call this, so the decision applies identically whether the
/// shake moves a camera or the whole page. Kept pure (no Bevy, no RNG) so
/// `cargo test` covers it without a GPU.
pub fn shake_magnitude(total_hull_damage: f32, intensity: f32) -> f32 {
    let saturated = (total_hull_damage / SHAKE_DAMAGE_FULL).clamp(0.0, 1.0);
    saturated * SHAKE_MAX_MAGNITUDE * intensity.clamp(0.0, 1.0)
}

/// Scale the shield-hit white flash by the endpoint's flash setting
/// (issue #1428, replacing issue #1173's reduced-motion cap).
///
/// `intensity` is [`ViewscreenMotion::flash_intensity`]: `1.0` passes the
/// decayed value through unchanged, `0.0` disables the flash outright, and the
/// values between are a genuinely dimmer jolt rather than a switch. The
/// reduced-motion preference reaches this the same way it reaches the shake —
/// as the default intensity a following effect takes — so the old behaviour
/// (no flash at all under reduce) is what an unpublished endpoint still gets.
pub fn scaled_flash_intensity(raw: f32, intensity: f32) -> f32 {
    raw * intensity.clamp(0.0, 1.0)
}

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
// message. The wasm forwarding to JS lives in `bridge::flush_host_channels`.

/// Single-entity component carrying the latest serialised HUD state. Bevy
/// change-detection drives the JS push.
#[derive(Component, Clone, PartialEq)]
struct ViewscreenHud(ViewscreenHudState);

/// Startup system: spawn the single entity that carries the HUD state.
fn spawn_hud_state_entity(mut commands: Commands) {
    commands.spawn(ViewscreenHud(ViewscreenHudState {
        heading: 0,
        hull_pct: 100,
        // Display-string id (issue #975); `localiseTree` resolves it on the
        // client. Rust never sends the English condition word.
        condition: "server.hud_nominal".to_string(),
        red_alert: false,
        engine_thrust: 0.0,
        phaser_firing: false,
        game_over_message: None,
        computer_message: None,
        game_over_report: Vec::new(),
        game_over_outcome: None,
        scenario_title: None,
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
    phaser_firing: bool,
    phase: &GamePhase,
    game_over_reason: Option<&GameOverReason>,
    // The active ship's-computer message, already reduced to its wire shape
    // (issue #1342). `None` clears the banner. Taken as an owned value rather
    // than the live resource so this stays a pure fn callable from a unit
    // test with no ECS at all — the same reason every other parameter here is
    // a plain value, not a `Query`/`Res`.
    computer_message: Option<ComputerMessageWire>,
    // The post-mission report (issue #1344). Read here rather than composed:
    // the rows are already the score-free player projection, and the Viewscreen
    // renders exactly what a phone does.
    mission_report: Option<&crate::core::report::MissionReport>,
    // The world's `[global] title` id. Published only at game over, where the
    // ending names the scenario it belongs to (PRD #1023 module 4) — the same
    // frame a phone draws from `lobbyState.scenarioTitle`.
    scenario_title: Option<&str>,
) -> ViewscreenHudState {
    let alert = red_alert;
    let hull_pct = if hull_max > 0.0 {
        (hull_current / hull_max * 100.0).clamp(0.0, 100.0)
    } else {
        0.0
    };
    // Pass the reason through untouched (issue #977). The built-in death sites
    // latch the `server.game_over.ship_destroyed` string id, and a scenario's
    // `game_over` action authors its own message id; either way this is a
    // `strings.csv` id (or authored data) the HUD channel resolves through
    // `localiseTree` client-side. Rust composes no display English here.
    let game_over_message = (*phase == GamePhase::GameOver).then(|| {
        game_over_reason
            .and_then(|r| r.0.clone())
            .unwrap_or_default()
    });
    // Empty until the game ends, and empty afterwards too for a scenario that
    // authored no report — which is what keeps every other world's Viewscreen
    // ending exactly as it was.
    let game_over_report = if *phase == GamePhase::GameOver {
        mission_report
            .map(|report| {
                report
                    .rows()
                    .iter()
                    .map(|row| crate::core::messages::GameOverReportRow {
                        id: row.id.clone(),
                        heading: row.heading_id.clone(),
                        outcome: row.outcome_id.clone(),
                        state: row.state.as_str().to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    // The declared side and the scenario's name travel with the ending only.
    // The outcome is `GameOverReason.1` — latched Defeat at the built-in death
    // sites, whatever a world's `game_over` action declared, or None — and is
    // NOT inferred from the reason text here, for the reason balance.rs gives:
    // the reason is per-world prose (or a strings.csv id) and no substring
    // reliably tells a win from a loss.
    let ended = *phase == GamePhase::GameOver;
    let game_over_outcome = if ended {
        game_over_reason
            .and_then(|r| r.1)
            .map(|o| o.as_str().to_string())
    } else {
        None
    };
    let scenario_title = if ended {
        scenario_title.map(str::to_owned)
    } else {
        None
    };
    ViewscreenHudState {
        heading: yaw_to_compass_bearing(physics.yaw),
        hull_pct: hull_pct.round() as i32,
        // Display-string ids (issue #975); the client resolves them through
        // `localiseTree`. No player-visible English is composed here.
        condition: if alert {
            "server.hud_alert"
        } else {
            "server.hud_nominal"
        }
        .to_string(),
        red_alert: alert,
        engine_thrust,
        phaser_firing,
        game_over_message,
        computer_message,
        game_over_report,
        game_over_outcome,
        scenario_title,
    }
}

/// Reduce the authoritative [`crate::core::computer_message::ComputerMessageState`]
/// to its Viewscreen wire shape (issue #1342). `text` and `station` stay
/// `strings.csv`/authored ids — the HUD channel resolves `text` through
/// `localiseHostPayload`/`t()` exactly as `game_over_message` already does.
fn to_computer_message_wire(
    state: &crate::core::computer_message::ComputerMessageState,
) -> ComputerMessageWire {
    ComputerMessageWire {
        id: state.id.clone(),
        text: state.text.clone(),
        severity: state.severity.as_str().to_string(),
        station: state.station.as_ref().map(|s| s.0.clone()),
    }
}

/// Per-frame system: recompute the HUD state and write it into the
/// `ViewscreenHud` component only when it differs, so `Changed<ViewscreenHud>`
/// fires only on actual change.
fn recompute_hud_state(
    red_alert_q: Query<&crate::ship::state::ShipRedAlert, With<crate::server_app::LocalShip>>,
    hull_q: Query<&crate::entities::spawner::EntitySystemHull, With<crate::server_app::LocalShip>>,
    phase: Option<Res<State<GamePhase>>>,
    game_over_reason: Option<Res<GameOverReason>>,
    mission_report: Option<Res<crate::core::report::MissionReport>>,
    physics_q: Query<&ShipPhysics, With<crate::server_app::LocalShip>>,
    last_input_q: Query<&crate::ship_plugin::LastHelmInput, With<crate::server_app::LocalShip>>,
    beam_q: Query<&crate::console::weapons::ActiveBeam, With<crate::server_app::LocalShip>>,
    computer_message: Option<Res<ActiveComputerMessage>>,
    world_resource: Option<Res<WorldResource>>,
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
    // `ActiveBeam::is_firing` is true while ANY bank is burning (issue #790) —
    // the HUD's "phasers are firing" hum is a ship-level state, not a per-bank
    // one, so two live broadsides read the same as one.
    let phaser_firing = beam_q.single().map(|b| b.is_firing()).unwrap_or(false);
    let computer_message_wire = computer_message
        .as_deref()
        .and_then(|m| m.current.as_ref())
        .map(to_computer_message_wire);
    let next = compute_hud_state(
        red_alert,
        &physics,
        hull_current,
        hull_max,
        engine_thrust,
        phaser_firing,
        phase.get(),
        game_over_reason.as_deref(),
        computer_message_wire,
        mission_report.as_deref(),
        world_resource
            .as_deref()
            .map(|w| w.0.scenario_title.as_str()),
    );
    for mut hud in hud_q.iter_mut() {
        if hud.0 != next {
            hud.0 = next.clone();
        }
    }
}

/// `OnEnter(GamePhase::GameOver)` system: push one final HUD state.
fn push_game_over_hud_state(
    red_alert_q: Query<&crate::ship::state::ShipRedAlert, With<crate::server_app::LocalShip>>,
    hull_q: Query<&crate::entities::spawner::EntitySystemHull, With<crate::server_app::LocalShip>>,
    game_over_reason: Option<Res<GameOverReason>>,
    mission_report: Option<Res<crate::core::report::MissionReport>>,
    world_resource: Option<Res<WorldResource>>,
    physics_q: Query<&ShipPhysics, With<crate::server_app::LocalShip>>,
    mut hud_q: Query<&mut ViewscreenHud>,
    mut writer: MessageWriter<HudStateChanged>,
) {
    let physics = physics_q.single().ok().copied().unwrap_or_default();
    let red_alert = red_alert_q.single().map(|ra| ra.0).unwrap_or(false);
    let (hull_current, hull_max) = hull_q
        .single()
        .map(|h| (h.0.total_current(), h.0.total_max()))
        .unwrap_or((100.0, 100.0));
    // Engine thrust and phaser fire are both forced off at game over so the
    // looping SFX stop with the sim. The computer-message banner is forced
    // off too (issue #1342 AC2: mission end clears it) — this push does not
    // wait on `clear_active_computer_message`'s own `ResMut` to land first,
    // it simply never shows one on the final HUD state. The post-mission
    // report (issue #1344) is the opposite case and is read live: the ending
    // is exactly when the rows have to be on screen.
    let next = compute_hud_state(
        red_alert,
        &physics,
        hull_current,
        hull_max,
        0.0,
        false,
        &GamePhase::GameOver,
        game_over_reason.as_deref(),
        None,
        mission_report.as_deref(),
        world_resource
            .as_deref()
            .map(|w| w.0.scenario_title.as_str()),
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

    #[test]
    fn presentation_readiness_requires_terminal_preload_but_not_a_renderer() {
        let mut preload = AssetPreloadResource::default();

        assert!(
            local_presentation_ready(None),
            "a rendererless host has no presentation preload to wait for"
        );
        assert!(
            !local_presentation_ready(Some(&preload)),
            "an unstarted or in-flight preload is not terminal"
        );

        preload.started = true;
        assert!(!local_presentation_ready(Some(&preload)));

        preload.complete = true;
        assert!(local_presentation_ready(Some(&preload)));
    }

    #[test]
    fn local_start_assets_wait_for_authoritative_rigs_without_a_renderer() {
        let rigs = crate::entities::model_markers::ModelRigReadiness::default();

        assert!(local_start_assets_ready(None, None));
        assert!(
            !local_start_assets_ready(None, Some(&rigs)),
            "rendererless does not mean canonical weapon geometry is ready"
        );

        let mut app = App::new();
        app.init_resource::<crate::entities::model_markers::ModelRigReadiness>()
            .add_systems(
                Update,
                crate::entities::model_markers::sync_authoritative_model_markers,
            );
        app.update();
        assert!(local_start_assets_ready(
            None,
            Some(
                app.world()
                    .resource::<crate::entities::model_markers::ModelRigReadiness>()
            )
        ));
    }

    #[test]
    fn host_lobby_roster_excludes_auxiliary_stations() {
        use crate::core::messages::StationId;
        use crate::lobby::stations_config::StationDef;

        let station = |id: &str, name: &str, auxiliary: bool| StationDef {
            id: StationId(id.into()),
            name: name.into(),
            description: String::new(),
            rank: String::new(),
            short_code: name.chars().take(3).collect(),
            console: None,
            ratings: vec!["Std".into()],
            human_seeking: auxiliary,
            host_order: vec![],
            visiting_rating: auxiliary.then(|| "Std".into()),
            auxiliary,
            command_target: None,
        };
        let stations = ShipStations {
            stations: vec![
                station("captain", "Captain", false),
                station("command", "Command", true),
            ],
        };
        let players = vec![
            Player {
                token: "captain-token".into(),
                name: "Ada".into(),
                connected: true,
                ready: true,
                station: Some(StationId("captain".into())),
                last_rating: None,
                spectator: false,
                afk: false,
            },
            Player {
                token: "auxiliary-token".into(),
                name: "Grace".into(),
                connected: true,
                ready: false,
                station: Some(StationId("command".into())),
                last_rating: None,
                spectator: false,
                afk: false,
            },
        ];

        let roster = claimable_lobby_roster(&stations, &players);

        assert_eq!(
            roster.stations.len(),
            1,
            "only the claimable seat becomes a card"
        );
        assert_eq!(roster.stations[0].name, "Captain");
        assert_eq!(roster.stations[0].holder_name.as_deref(), Some("Ada"));
        assert!(roster
            .stations
            .iter()
            .all(|payload| payload.name != "Command"));
        assert_eq!(
            roster.max_players, 1,
            "auxiliary Stations are not lobby slots"
        );
        assert_eq!(
            roster.crew_count, 1,
            "crew counts only claimable held seats"
        );
        assert!(
            roster.all_filled,
            "an empty auxiliary Station cannot block filled state"
        );
    }

    // ── motion comfort: shake_magnitude (issues #1173, #1428) ────────

    #[test]
    fn shake_scales_with_damage_and_saturates() {
        // Full intensity: linear up to the full-hit threshold, then clamped to
        // the max magnitude.
        let half = shake_magnitude(SHAKE_DAMAGE_FULL / 2.0, 1.0);
        assert!((half - SHAKE_MAX_MAGNITUDE * 0.5).abs() < 1e-6);
        let full = shake_magnitude(SHAKE_DAMAGE_FULL, 1.0);
        assert!((full - SHAKE_MAX_MAGNITUDE).abs() < 1e-6);
        // Beyond the threshold it does not keep growing.
        let over = shake_magnitude(SHAKE_DAMAGE_FULL * 10.0, 1.0);
        assert!((over - SHAKE_MAX_MAGNITUDE).abs() < 1e-6);
    }

    #[test]
    fn shake_off_is_exactly_zero_at_any_damage() {
        // Issue #1428: "off" is an intensity of zero and it is absolute — the
        // AC1 guarantee for BOTH render paths, now reached by one number rather
        // than by a flag beside a scale.
        assert_eq!(shake_magnitude(SHAKE_DAMAGE_FULL, 0.0), 0.0);
        assert_eq!(shake_magnitude(SHAKE_DAMAGE_FULL * 100.0, 0.0), 0.0);
        assert_eq!(shake_magnitude(5.0, 0.0), 0.0);
    }

    #[test]
    fn shake_intensity_dials_between_full_and_off() {
        // The gentler stop is a genuinely smaller movement, not a switch.
        let base = shake_magnitude(SHAKE_DAMAGE_FULL, 1.0);
        let half = shake_magnitude(SHAKE_DAMAGE_FULL, 0.5);
        assert!((half - base * 0.5).abs() < 1e-6);
        // Out-of-range intensity is clamped, never amplified past the max.
        let over = shake_magnitude(SHAKE_DAMAGE_FULL, 4.0);
        assert!((over - SHAKE_MAX_MAGNITUDE).abs() < 1e-6);
    }

    #[test]
    fn no_damage_no_shake() {
        assert_eq!(shake_magnitude(0.0, 1.0), 0.0);
    }

    // ── motion comfort: scaled_flash_intensity (issues #1173, #1428) ─

    #[test]
    fn flash_passes_through_at_full_intensity() {
        // Nothing asked for less: the decayed value is untouched.
        assert_eq!(scaled_flash_intensity(1.0, 1.0), 1.0);
        assert_eq!(scaled_flash_intensity(0.42, 1.0), 0.42);
        assert_eq!(scaled_flash_intensity(0.0, 1.0), 0.0);
    }

    #[test]
    fn flash_off_removes_the_jolt_entirely() {
        assert_eq!(scaled_flash_intensity(1.0, 0.0), 0.0);
        assert_eq!(scaled_flash_intensity(0.7, 0.0), 0.0);
    }

    #[test]
    fn flash_intensity_dims_rather_than_switching() {
        // Issue #1428 replaced the all-or-nothing cap with a scale, so the
        // gentler stop still shows a shield hit — dimmer, not absent. That is
        // the "essential feedback survives a reduced effect" half of story 15.
        let dimmed = scaled_flash_intensity(1.0, 0.3);
        assert!((dimmed - 0.3).abs() < 1e-6);
        assert!(dimmed > 0.0, "a gentler flash is still a visible flash");
        // Out-of-range intensity cannot amplify the jolt.
        assert_eq!(scaled_flash_intensity(1.0, 4.0), 1.0);
    }

    #[test]
    fn viewscreen_motion_default_is_full_normal_motion() {
        let m = ViewscreenMotion::default();
        assert!(!m.reduced_motion);
        assert_eq!(m.shake_intensity, DEFAULT_SHAKE_INTENSITY);
        assert_eq!(m.flash_intensity, DEFAULT_FLASH_INTENSITY);
        // The default must leave both effects untouched from the pre-#1173
        // formulas.
        assert!(
            (shake_magnitude(SHAKE_DAMAGE_FULL, m.shake_intensity) - SHAKE_MAX_MAGNITUDE).abs()
                < 1e-6
        );
        assert_eq!(scaled_flash_intensity(0.8, m.flash_intensity), 0.8);
    }

    #[test]
    fn following_the_preference_is_what_reduced_motion_used_to_do() {
        // The resolution `sync_viewscreen_motion` performs, stated as the rule
        // it is: an effect that has published nothing takes the preference's
        // default, and that default under reduce is exactly zero — so a build
        // whose page only ever calls `wasm_set_reduced_motion` behaves as it did
        // before issue #1428 split the lever in three.
        let following = |reduced: bool| {
            if reduced {
                REDUCED_MOTION_INTENSITY
            } else {
                DEFAULT_SHAKE_INTENSITY
            }
        };
        assert_eq!(shake_magnitude(SHAKE_DAMAGE_FULL, following(true)), 0.0);
        assert_eq!(scaled_flash_intensity(1.0, following(true)), 0.0);
        assert!(shake_magnitude(SHAKE_DAMAGE_FULL, following(false)) > 0.0);
        assert_eq!(scaled_flash_intensity(1.0, following(false)), 1.0);
    }

    #[test]
    fn an_explicit_intensity_outranks_the_preference_in_both_directions() {
        // The point of the `Option`: a published value is the operator's, and a
        // published `0.0` is a choice rather than an absence. Both are honoured
        // over whatever the machine's preference would have defaulted to.
        let resolve = |published: Option<f32>, reduced: bool| {
            let following = if reduced {
                REDUCED_MOTION_INTENSITY
            } else {
                DEFAULT_SHAKE_INTENSITY
            };
            published.unwrap_or(following).clamp(0.0, 1.0)
        };
        // Keep the shake on a machine whose OS asked to reduce motion.
        assert!(shake_magnitude(SHAKE_DAMAGE_FULL, resolve(Some(1.0), true)) > 0.0);
        // Turn it off on a machine whose OS asked for nothing.
        assert_eq!(
            shake_magnitude(SHAKE_DAMAGE_FULL, resolve(Some(0.0), false)),
            0.0
        );
        // Publishing nothing follows.
        assert_eq!(shake_magnitude(SHAKE_DAMAGE_FULL, resolve(None, true)), 0.0);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_decorative_band_the_hud_overlay_is_stamped_from_follows_the_same_rule() {
        // The third effect has no uniform and no camera, but the native
        // Viewscreen still has to be told about it: its frame, readout and
        // red-alert vignette are a separate DOCUMENT, and
        // `panes::ultralight::cache_hud_state` stamps that document from this
        // resource. Resolved here, beside the two the renderer owns, so a
        // display following the machine cannot end up with a glow that
        // disagrees with the shader behind it.
        use crate::server::bridge::{
            clear_native_effect_intensities, set_native_effect_intensities,
            NATIVE_EFFECT_LATCH_TEST_LOCK,
        };
        use bevy::ecs::system::RunSystemOnce;

        let _serialised = NATIVE_EFFECT_LATCH_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        clear_native_effect_intensities();

        let mut app = App::new();
        app.init_resource::<ViewscreenMotion>();
        let resolved = |app: &mut App| {
            app.world_mut().run_system_once(sync_viewscreen_motion).ok();
            app.world().resource::<ViewscreenMotion>().clone()
        };

        // Nothing published, and a machine that asked for nothing: full.
        let motion = resolved(&mut app);
        assert_eq!(motion.decorative_intensity, DEFAULT_DECORATIVE_INTENSITY);

        // An explicit off is the operator's, and beats the machine either way.
        set_native_effect_intensities(Some(100), Some(100), Some(0));
        assert_eq!(resolved(&mut app).decorative_intensity, 0.0);

        // Following, on a machine that asked to reduce, is exactly zero — what
        // `data-reduced-motion` did on its own before the lever was split.
        clear_native_effect_intensities();
        app.world_mut()
            .resource_mut::<ViewscreenMotion>()
            .reduced_motion = true;
        assert_eq!(
            resolved(&mut app).decorative_intensity,
            REDUCED_MOTION_INTENSITY
        );

        // …and an explicit KEEP survives that same machine, which is the
        // direction a one-lever build could not express.
        set_native_effect_intensities(None, None, Some(100));
        assert_eq!(resolved(&mut app).decorative_intensity, 1.0);

        clear_native_effect_intensities();
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_native_latch_carries_a_choice_and_a_reset() {
        use crate::server::bridge::{
            published_effect_intensities, set_native_effect_intensities,
            NATIVE_EFFECT_LATCH_TEST_LOCK,
        };

        // The latch is process-global, and the host-lobby test that seeds it
        // from a saved record shares it; one lock keeps the two off each other.
        let _serialised = NATIVE_EFFECT_LATCH_TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        // Whole percent in, fractions out — the shape the setting already
        // crosses the page/host bridge in (issue #1428).
        set_native_effect_intensities(Some(30), Some(0), Some(40));
        let (shake, flash, decorative) = published_effect_intensities();
        assert!((shake.expect("a published shake") - 0.3).abs() < 1e-6);
        assert_eq!(
            flash,
            Some(0.0),
            "a published zero is a choice, not an absence"
        );
        // The third effect rides the same latch but is nobody's uniform: it is
        // the band the native HUD overlay's document is stamped with.
        assert!((decorative.expect("a published band") - 0.4).abs() < 1e-6);

        // A per-setting reset publishes nothing again, so the effect goes back
        // to following the machine rather than sticking at its last number.
        set_native_effect_intensities(None, None, None);
        assert_eq!(published_effect_intensities(), (None, None, None));
    }

    // ── compute_hud_state ────────────────────────────────────────────

    #[test]
    fn compute_hud_state_nominal() {
        let physics = ShipPhysics::default();
        let state = compute_hud_state(
            false,
            &physics,
            100.0,
            100.0,
            0.0,
            false,
            &GamePhase::InProgress,
            None,
            None,
            None,
            None,
        );
        assert_eq!(state.heading, 0);
        assert_eq!(state.hull_pct, 100);
        // The condition rides the wire as a string id (issue #975), resolved on
        // the client; Rust's contract is the id, not the English word.
        assert_eq!(state.condition, "server.hud_nominal");
        assert!(!state.red_alert);
        assert_eq!(state.engine_thrust, 0.0);
        assert!(!state.phaser_firing);
        assert!(state.game_over_message.is_none());
    }

    #[test]
    fn compute_hud_state_alert_and_partial_hull() {
        let physics = ShipPhysics {
            yaw: std::f32::consts::FRAC_PI_2,
            ..Default::default()
        };
        let state = compute_hud_state(
            true,
            &physics,
            50.0,
            100.0,
            0.75,
            true,
            &GamePhase::InProgress,
            None,
            None,
            None,
            None,
        );
        assert_eq!(state.heading, 90);
        assert_eq!(state.hull_pct, 50);
        assert_eq!(state.condition, "server.hud_alert");
        assert!(state.red_alert);
        assert!((state.engine_thrust - 0.75).abs() < f32::EPSILON);
        assert!(state.phaser_firing);
    }

    #[test]
    fn compute_hud_state_engine_thrust_propagated() {
        let physics = ShipPhysics::default();
        let state = compute_hud_state(
            false,
            &physics,
            100.0,
            100.0,
            0.5,
            false,
            &GamePhase::InProgress,
            None,
            None,
            None,
            None,
        );
        assert_eq!(state.engine_thrust, 0.5);
    }

    #[test]
    fn compute_hud_state_game_over_ship_destroyed() {
        use crate::server_app::GameOverReason;
        let physics = ShipPhysics::default();
        // The built-in death sites latch the string id (issue #977);
        // `compute_hud_state` passes it through, `localiseTree` resolves it to
        // "Ship Destroyed" on the client. No English is composed here.
        let reason = GameOverReason(Some("server.game_over.ship_destroyed".into()), None);
        let state = compute_hud_state(
            false,
            &physics,
            0.0,
            100.0,
            0.0,
            false,
            &GamePhase::GameOver,
            Some(&reason),
            None,
            None,
            None,
        );
        assert_eq!(
            state.game_over_message.as_deref(),
            Some("server.game_over.ship_destroyed")
        );
    }

    /// The Viewscreen frames the ending the way a phone does: the declared
    /// side and the scenario's title ride the final HUD state, and neither is
    /// published while the mission still runs.
    #[test]
    fn compute_hud_state_frames_the_ending_with_outcome_and_scenario() {
        use crate::server_app::GameOverReason;
        let physics = ShipPhysics::default();
        let reason = GameOverReason(
            Some("world.falling_skyway.game_over.mission_complete".into()),
            Some(crate::core::balance::Outcome::Victory),
        );
        let title = "world.falling_skyway.global.title";

        let live = compute_hud_state(
            false,
            &physics,
            80.0,
            100.0,
            0.0,
            false,
            &GamePhase::InProgress,
            Some(&reason),
            None,
            None,
            Some(title),
        );
        assert!(live.game_over_outcome.is_none());
        assert!(live.scenario_title.is_none());

        let ended = compute_hud_state(
            false,
            &physics,
            80.0,
            100.0,
            0.0,
            false,
            &GamePhase::GameOver,
            Some(&reason),
            None,
            None,
            Some(title),
        );
        // `balance::Outcome::as_str`, the spelling ServerMessage::GameOver uses.
        assert_eq!(ended.game_over_outcome.as_deref(), Some("victory"));
        // An id, resolved by the host channel; Rust composes no English here.
        assert_eq!(ended.scenario_title.as_deref(), Some(title));

        // An ending that declared no side publishes none: the frame decides
        // ENDED from that absence, never from the closing prose.
        let undeclared = GameOverReason(Some("The channel went quiet.".into()), None);
        let ended = compute_hud_state(
            false,
            &physics,
            80.0,
            100.0,
            0.0,
            false,
            &GamePhase::GameOver,
            Some(&undeclared),
            None,
            None,
            Some(title),
        );
        assert!(ended.game_over_outcome.is_none());
    }

    /// Issue #1344: the Viewscreen shows the SAME rows a phone does, in the
    /// same order, and carries no score for the same reason the phone's wire
    /// row has no field to put one in.
    #[test]
    fn compute_hud_state_carries_the_post_mission_report_at_game_over() {
        use crate::core::report::{MissionReport, ReportRow, ReportRowState};
        use crate::server_app::GameOverReason;

        let physics = ShipPhysics::default();
        let reason = GameOverReason(
            Some("world.falling_skyway.game_over.lark_collision".into()),
            Some(crate::core::balance::Outcome::Defeat),
        );
        let mut report = MissionReport::default();
        report.set_row(ReportRow {
            id: "lyra".into(),
            heading_id: "world.falling_skyway.report.lyra.heading".into(),
            outcome_id: "world.falling_skyway.report.lyra.saved".into(),
            state: ReportRowState::Saved,
            score: 6,
        });

        // While the mission runs there is nothing to report on yet, even though
        // the row is already written.
        let live = compute_hud_state(
            false,
            &physics,
            80.0,
            100.0,
            0.0,
            false,
            &GamePhase::InProgress,
            Some(&reason),
            None,
            Some(&report),
            None,
        );
        assert!(live.game_over_report.is_empty());

        let ended = compute_hud_state(
            false,
            &physics,
            80.0,
            100.0,
            0.0,
            false,
            &GamePhase::GameOver,
            Some(&reason),
            None,
            Some(&report),
            None,
        );
        assert_eq!(ended.game_over_report.len(), 1);
        assert_eq!(ended.game_over_report[0].id, "lyra");
        assert_eq!(
            ended.game_over_report[0].heading,
            "world.falling_skyway.report.lyra.heading"
        );
        assert_eq!(ended.game_over_report[0].state, "saved");
        // Every text field is a String Id the host channel localises; Rust
        // composes no English on this surface.
        assert!(ended.game_over_report[0]
            .outcome
            .starts_with("world.falling_skyway.report."));
    }

    /// A scenario that authored no report ends exactly as it always did.
    #[test]
    fn compute_hud_state_reports_nothing_for_a_scenario_without_a_report() {
        use crate::server_app::GameOverReason;
        let physics = ShipPhysics::default();
        let reason = GameOverReason(Some("server.game_over.ship_destroyed".into()), None);
        let state = compute_hud_state(
            false,
            &physics,
            0.0,
            100.0,
            0.0,
            false,
            &GamePhase::GameOver,
            Some(&reason),
            None,
            Some(&crate::core::report::MissionReport::default()),
            None,
        );
        assert!(state.game_over_report.is_empty());
        assert_eq!(
            state.game_over_message.as_deref(),
            Some("server.game_over.ship_destroyed")
        );
    }

    #[test]
    fn compute_hud_state_game_over_scenario_message() {
        use crate::server_app::GameOverReason;
        let physics = ShipPhysics::default();
        let reason = GameOverReason(Some("VICTORY: All enemies eliminated.".into()), None);
        let state = compute_hud_state(
            false,
            &physics,
            50.0,
            100.0,
            0.0,
            false,
            &GamePhase::GameOver,
            Some(&reason),
            None,
            None,
            None,
        );
        assert_eq!(
            state.game_over_message.as_deref(),
            Some("VICTORY: All enemies eliminated.")
        );
    }

    // ── computer_message passthrough (issue #1342) ────────────────────

    #[test]
    fn compute_hud_state_carries_the_active_computer_message() {
        let physics = ShipPhysics::default();
        let msg = ComputerMessageWire {
            id: "hail_debris".into(),
            text: "world.probe.computer_message.text".into(),
            severity: "advisory".into(),
            station: Some("tactical".into()),
        };
        let state = compute_hud_state(
            false,
            &physics,
            100.0,
            100.0,
            0.0,
            false,
            &GamePhase::InProgress,
            None,
            Some(msg.clone()),
            None,
            None,
        );
        assert_eq!(state.computer_message, Some(msg));
    }

    #[test]
    fn to_computer_message_wire_reduces_the_authoritative_state() {
        use crate::core::computer_message::{ComputerMessageSeverity, ComputerMessageState};
        use crate::core::messages::StationId;
        let state = ComputerMessageState {
            id: "charge_ready".into(),
            text: "world.probe.computer_message.charge".into(),
            severity: ComputerMessageSeverity::Critical,
            station: Some(StationId("tactical".into())),
            shown_tick: 0,
            expires_tick: 480,
        };
        let wire = to_computer_message_wire(&state);
        assert_eq!(wire.id, "charge_ready");
        assert_eq!(wire.text, "world.probe.computer_message.charge");
        assert_eq!(wire.severity, "critical");
        assert_eq!(wire.station.as_deref(), Some("tactical"));
    }

    #[test]
    fn game_over_hud_push_never_carries_a_computer_message() {
        // AC2: mission end clears the banner. The final push forces `None`
        // regardless of what the resource holds, rather than racing
        // `clear_active_computer_message`'s own `OnEnter` system.
        let physics = ShipPhysics::default();
        let state = compute_hud_state(
            false,
            &physics,
            0.0,
            100.0,
            0.0,
            false,
            &GamePhase::GameOver,
            None,
            None,
            None,
            None,
        );
        assert!(state.computer_message.is_none());
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
