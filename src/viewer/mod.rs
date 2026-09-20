//! Shared Workshop model / shader preview (`--features viewer`).
//!
//! A dev tool for iterating on how things look without booting the lobby,
//! joining a scenario, and flying somewhere. It renders **one** subject through
//! the same shared setup the game uses — [`crate::render_setup`] for the
//! skybox, camera optics and ambient fill; [`crate::entities::glb_visual`] for
//! GLB + `.model.toml` rig composition; [`crate::entities::celestial_visual`]
//! for the star/planet WGSL materials. Nothing here reimplements the render
//! path; if it did, the tool would stop being a valid reference.
//!
//! Selection is owned by Workshop's versioned launch descriptor. This module
//! contains renderer state and controls, not a second URL/config vocabulary.

use bevy::prelude::*;
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
use crate::render_setup::SPACE_SKYBOX_BRIGHTNESS;
use crate::render_setup::{
    game_camera_projection, space_skybox, SpaceSkyboxAsset, SpaceSkyboxPlugin,
};

mod camera;
mod capture;
mod gizmos;
mod lighting;
mod lod;
/// The Workshop's disposable preview: this same plugin over captured bytes.
pub mod preview;
mod stats;
mod subject;

pub use camera::OrbitCamera;
pub use lighting::LightingMode;
pub use lod::{LadderState, LodMode};

pub(crate) fn reset_pack_visuals(world: &mut World) {
    use bevy::ecs::system::RunSystemOnce;
    if !world.contains_resource::<subject::SubjectState>() {
        return;
    }
    if let Some(mut ladder) = world.get_resource_mut::<LadderState>() {
        *ladder = LadderState::default();
    }
    world
        .run_system_once(
            |mut commands: Commands, mut state: ResMut<subject::SubjectState>| {
                state.showing = subject::Showing::Base;
                state.respawn(&mut commands);
            },
        )
        .expect("viewer reset owns its existing subject");
}

/// Parsed URL parameters, resolved once at startup.
#[derive(Resource, Debug, Clone)]
pub struct ViewerArgs {
    pub model: Option<String>,
    pub variant: Option<String>,
    pub entity: Option<String>,
    pub gizmos: bool,
}

impl Default for ViewerArgs {
    fn default() -> Self {
        Self {
            model: Some("assets/models/alliance_cruiser.glb".to_string()),
            variant: None,
            entity: None,
            gizmos: false,
        }
    }
}

/// Commands queued from JS (the HTML control panel) and drained each frame.
///
/// The panel runs on the JS side rather than in Bevy UI so tweaking a slider
/// costs an HTML edit, not a wasm rebuild.
#[derive(Debug, Clone)]
pub enum ViewerCommand {
    SetLighting(LightingMode),
    SetAmbient {
        color: [f32; 3],
        brightness: f32,
    },
    SetDirectional {
        illuminance: f32,
        yaw: f32,
        pitch: f32,
    },
    SetSkyboxBrightness(f32),
    SetGizmos(bool),
    LoadModel {
        path: String,
        variant: Option<String>,
    },
    /// Render a complete entity template rather than a bare GLB. This is how
    /// the viewer reaches authored star and planet materials without creating a
    /// second celestial render path.
    LoadEntity {
        path: String,
    },
    SetLodMode(LodMode),
    /// Put the camera at a given distance from the subject — how the panel
    /// jumps to a LOD band's switch distance.
    SetCameraDistance(f32),
    /// Restore the complete orbit state after a live-reload.  Distance alone
    /// loses the user's orbit heading and pan offset.
    SetCamera {
        focus: Vec3,
        radius: f32,
        yaw: f32,
        pitch: f32,
    },
    /// Replace the ladder with the panel's working copy, so an edited switch
    /// distance takes effect before it is saved to the sidecar.
    SetLadder(Vec<crate::entities::config::LodLevel>),
    /// Re-fetch every asset this ladder names, then rebuild the subject from
    /// what came back.
    ReloadAssets,
    /// Bake the far-LOD billboard: render the subject to a transparent yaw-ring
    /// atlas the panel then saves. See [`capture`].
    CaptureBillboard {
        views: u32,
        resolution: u32,
        pitch_deg: f32,
    },
}

// The panel builds a ladder one level at a time rather than handing over a
// serialised one: `serde_json` is confined to codec.rs (Key Constraint 1), and
// three exports with plain scalar arguments need no wire format at all.
thread_local! {
    static LADDER_DRAFT: RefCell<Vec<crate::entities::config::LodLevel>> =
        const { RefCell::new(Vec::new()) };
}

thread_local! {
    static COMMAND_QUEUE: RefCell<Vec<ViewerCommand>> = const { RefCell::new(Vec::new()) };
}

// Every caller is a `#[wasm_bindgen]` export gated on `target_arch = "wasm32"`,
// so on a native build (CI's clippy target) this is genuinely unreachable —
// gate it to match its callers rather than silencing dead_code.
#[cfg(target_arch = "wasm32")]
fn push_command(cmd: ViewerCommand) {
    COMMAND_QUEUE.with(|q| q.borrow_mut().push(cmd));
}

fn drain_commands() -> Vec<ViewerCommand> {
    COMMAND_QUEUE.with(|q| std::mem::take(&mut *q.borrow_mut()))
}

/// Marker for the viewer's single 3D camera.
#[derive(Component)]
pub struct ViewerCamera;

/// Closest the panel may put the camera. Zero would put it inside the subject
/// with nothing to look at and no way to scroll back out.
const MIN_CAMERA_DISTANCE: f32 = 0.1;

/// The shared preview systems are testable and buildable off-wasm.
pub struct ViewerPlugin {
    pub args: ViewerArgs,
}

impl Plugin for ViewerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.args.clone())
            // The same skybox + custom-material plugins the game registers, so
            // every shader the viewer can show is the shader the game shows.
            .add_plugins(SpaceSkyboxPlugin)
            .add_plugins(crate::entities::star::StarRenderPlugin)
            .add_plugins(crate::entities::planet::PlanetRenderPlugin)
            .init_resource::<LightingMode>()
            .init_resource::<crate::entities::planet::PlanetLightingOverride>()
            .init_resource::<subject::SubjectState>()
            .init_resource::<lod::LadderState>()
            .init_resource::<lod::LodMode>()
            .init_resource::<stats::SubjectStats>()
            .init_resource::<capture::CaptureRequest>()
            .init_resource::<capture::CaptureState>()
            .init_resource::<crate::server_app::ProceduralMeshCache>()
            .add_systems(Startup, (setup_camera, subject::spawn_subject).chain())
            .add_systems(
                Update,
                (
                    // Ordered: a command can change the model, which changes the
                    // ladder, which changes the level, which is what gets
                    // spawned — all in one frame rather than one step per frame.
                    apply_commands,
                    lod::refresh_ladder,
                    lod::apply_lod_mode,
                    subject::poll_pending_model,
                    subject::respawn_on_asset_reload,
                    stats::measure_subject,
                    stats::publish_stats,
                    // After `measure_subject` so the framing extents are known.
                    capture::start_capture,
                    capture::drive_capture,
                    capture::publish_capture,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    lighting::apply_lighting,
                    camera::orbit_camera,
                    camera::frame_subject_once,
                    gizmos::draw_rig_gizmos,
                    // The game's own billboard system, driven off the viewer's
                    // camera. This is what makes a previewed imposter turn and
                    // blend the way it does in game rather than hanging at tile
                    // 0 — the whole point of previewing it here at all (PRD
                    // #1023, module 5).
                    crate::entities::billboard::orient_lod_billboards::<ViewerCamera>,
                ),
            );
    }
}

fn setup_camera(mut commands: Commands, skybox: Res<SpaceSkyboxAsset>) {
    let camera = commands
        .spawn((
            ViewerCamera,
            Camera3d::default(),
            game_camera_projection(),
            space_skybox(&skybox),
            OrbitCamera::default(),
            Transform::default(),
        ))
        .id();
    // The same HDR / tonemapping / bloom calibration the game camera takes
    // (PRD #1023). The viewer exists to be a valid reference for how things
    // look; one that resolved light differently from the viewscreen would be a
    // tool for tuning a picture nobody sees.
    //
    // That parity now includes the platform gate, and it costs nothing to state:
    // the viewer is a Trunk/WASM page on WebGL2 exactly like the game host, so
    // `BLOOM_RUNS_ON_THIS_TARGET` is false in every build of it that exists and
    // the camera gets HDR and the display transform but no bloom pass — the same
    // picture the viewscreen shows, which is the whole point of this call. A
    // native viewer would get bloom here without a line changing, because the
    // gate lives in `apply_render_config` rather than in either camera's setup.
    crate::render_setup::apply_render_config(
        &mut commands,
        camera,
        &crate::world::config::RenderConfig::default(),
    );
}

/// Drain the JS command queue and apply each command to the world.
fn apply_commands(
    mut lighting: ResMut<LightingMode>,
    mut args: ResMut<ViewerArgs>,
    mut subject_state: ResMut<subject::SubjectState>,
    mut lod_mode: ResMut<LodMode>,
    mut ladder: ResMut<LadderState>,
    asset_server: Res<AssetServer>,
    mut skyboxes: Query<&mut bevy::core_pipeline::Skybox>,
    mut cameras: Query<&mut OrbitCamera>,
    mut capture_request: ResMut<capture::CaptureRequest>,
    mut commands: Commands,
) {
    for cmd in drain_commands() {
        match cmd {
            ViewerCommand::SetLighting(mode) => *lighting = mode,
            ViewerCommand::SetAmbient { color, brightness } => {
                lighting.ambient_color = color;
                lighting.ambient_brightness = brightness;
            }
            ViewerCommand::SetDirectional {
                illuminance,
                yaw,
                pitch,
            } => {
                lighting.directional_illuminance = illuminance;
                lighting.directional_yaw = yaw;
                lighting.directional_pitch = pitch;
            }
            ViewerCommand::SetSkyboxBrightness(b) => {
                for mut skybox in &mut skyboxes {
                    skybox.brightness = b;
                }
            }
            ViewerCommand::SetGizmos(on) => args.gizmos = on,
            ViewerCommand::LoadModel { path, variant } => {
                args.model = Some(path);
                args.variant = variant;
                args.entity = None;
                subject_state.showing = subject::Showing::Base;
                // A different model wants framing again — a 12 m courier and a
                // 400 m starbase do not share a usable camera distance. Only an
                // explicit model switch does this: a LOD swap must leave the
                // camera exactly where the person put it, or the ladder could
                // never be judged at a fixed distance.
                for mut orbit in &mut cameras {
                    orbit.framed = false;
                }
                subject_state.respawn(&mut commands);
            }
            ViewerCommand::LoadEntity { path } => {
                args.model = None;
                args.variant = None;
                args.entity = Some(path);
                subject_state.showing = subject::Showing::Base;
                // Entity templates do not own a model LOD ladder. Clear the
                // previous model immediately, rather than leaving its levels
                // live until `refresh_ladder` observes `model = None`.
                *ladder = lod::LadderState::default();
                *lod_mode = LodMode::Base;
                for mut orbit in &mut cameras {
                    orbit.framed = false;
                }
                subject_state.respawn(&mut commands);
            }
            ViewerCommand::SetLodMode(mode) => *lod_mode = mode,
            ViewerCommand::SetCameraDistance(distance) => {
                for mut orbit in &mut cameras {
                    // Recentre on the subject, or this does not mean what it
                    // says. The camera sits at `focus + rotation * radius`, and
                    // the distance every other part of the tool talks about —
                    // the readout, and the `select_lod` band — is measured from
                    // the SUBJECT at the origin. Right-drag panning moves
                    // `focus`, so setting the radius alone put the camera that
                    // far from wherever you had panned to: "range 5" landed
                    // inside the model or half a map away depending on history.
                    orbit.focus = Vec3::ZERO;
                    orbit.radius = distance.max(MIN_CAMERA_DISTANCE);
                    // Distance came from the panel, so framing must not
                    // overwrite it when the next level's extents arrive.
                    orbit.framed = true;
                }
            }
            ViewerCommand::SetCamera {
                focus,
                radius,
                yaw,
                pitch,
            } => {
                for mut orbit in &mut cameras {
                    orbit.focus = focus;
                    orbit.radius = radius.max(MIN_CAMERA_DISTANCE);
                    orbit.yaw = yaw;
                    orbit.pitch = pitch.clamp(
                        -std::f32::consts::FRAC_PI_2 + 0.01,
                        std::f32::consts::FRAC_PI_2 - 0.01,
                    );
                    orbit.framed = true;
                }
            }
            ViewerCommand::SetLadder(levels) => {
                // Only the levels change: `source` still names the model these
                // belong to, so the sidecar is not re-read and this edit
                // survives until the model itself changes.
                ladder.preloaded = lod::preload_levels(&asset_server, &levels);
                ladder.levels = levels;
            }
            ViewerCommand::ReloadAssets => {
                // Every path this ladder can show, plus the base model — the
                // whole set the panel might be looking at after a run.
                let mut paths: Vec<String> = ladder
                    .levels
                    .iter()
                    .filter_map(|level| level.model.clone())
                    .collect();
                paths.extend(args.model.clone());
                for path in paths {
                    asset_server.reload(crate::entities::pack_assets::asset_path(
                        &asset_server,
                        &path,
                    ));
                }
                // The rebuild waits for the new bytes; see
                // `respawn_on_asset_reload`. Respawning now would rebuild from
                // the very assets being replaced.
                subject_state.reloading = true;
            }
            ViewerCommand::CaptureBillboard {
                views,
                resolution,
                pitch_deg,
            } => {
                // Handed to `capture::start_capture`, which waits for the
                // subject's extents before it begins.
                capture_request.0 = Some(capture::CaptureParams {
                    views,
                    resolution,
                    pitch_deg,
                });
            }
        }
        lighting.set_changed();
    }
}

// ── JS control surface ─────────────────────────────────────────────────────

// Workshop preview has no server bridge. Export its own thin bindings
// over the shared content cache so rig and entity fetches work in that build.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_set_world_fetch_callback(callback: js_sys::Function) {
    crate::entities::config_cache::set_world_fetch_callback(callback);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_push_sidecar_toml(path: String, toml_str: String) -> bool {
    crate::entities::config_cache::wasm_push_sidecar_toml(path, toml_str)
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_set_lighting(mode: &str) {
    let mut lighting = LightingMode::default();
    lighting.mode = lighting::Mode::parse(mode);
    push_command(ViewerCommand::SetLighting(lighting));
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_set_ambient(r: f32, g: f32, b: f32, brightness: f32) {
    push_command(ViewerCommand::SetAmbient {
        color: [r, g, b],
        brightness,
    });
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_set_directional(illuminance: f32, yaw: f32, pitch: f32) {
    push_command(ViewerCommand::SetDirectional {
        illuminance,
        yaw,
        pitch,
    });
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_set_skybox_brightness(brightness: f32) {
    push_command(ViewerCommand::SetSkyboxBrightness(brightness));
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_set_gizmos(on: bool) {
    push_command(ViewerCommand::SetGizmos(on));
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_load_model(path: String, variant: Option<String>) {
    push_command(ViewerCommand::LoadModel {
        path,
        variant: variant.filter(|v| !v.is_empty()),
    });
}

/// Switch the viewer to an entity TOML. The subject renderer dispatches its
/// `[star]`, `[planet]`, or `[mesh]` visual through the same constructors used
/// by the game.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_load_entity(path: String) {
    push_command(ViewerCommand::LoadEntity { path });
}

/// The game's skybox brightness, so the HTML panel can seed its slider from
/// the real value rather than a duplicated constant.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_default_skybox_brightness() -> f32 {
    SPACE_SKYBOX_BRIGHTNESS
}

/// Choose how the ladder is applied: `base`, `auto`, or `fixed` with `index`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_set_lod_mode(mode: &str, index: usize) {
    push_command(ViewerCommand::SetLodMode(LodMode::parse(mode, index)));
}

/// Put the camera this far from the subject — the panel's "show me this band"
/// button, and the only way to reach a switch distance exactly.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_set_camera_distance(distance: f32) {
    push_command(ViewerCommand::SetCameraDistance(distance));
}

/// Restore the orbit heading, pan, and distance saved by the viewer panel
/// before Trunk reloaded the page.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_set_camera(
    focus_x: f32,
    focus_y: f32,
    focus_z: f32,
    radius: f32,
    yaw: f32,
    pitch: f32,
) {
    push_command(ViewerCommand::SetCamera {
        focus: Vec3::new(focus_x, focus_y, focus_z),
        radius,
        yaw,
        pitch,
    });
}

/// Start a new ladder draft. Levels are pushed one at a time and applied by
/// [`viewer_ladder_commit`], so an edit in the panel takes effect on the next
/// frame without a save, a reload, or a wire format.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_ladder_begin() {
    LADDER_DRAFT.with(|draft| draft.borrow_mut().clear());
}

/// Append one level to the draft.
///
/// `max_distance` is `f32::INFINITY` for the unbounded fallback level; `model`,
/// `variant` and `shape` are empty strings when absent. `scale` of `1,1,1` and
/// `rotation` of `0,0,0` mean "none of its own". A negative `colour_r` means
/// the level declares no colour and inherits the entity's.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_ladder_push(
    max_distance: f32,
    model: &str,
    variant: &str,
    shape: &str,
    scale_x: f32,
    scale_y: f32,
    scale_z: f32,
    rotation_x: f32,
    rotation_y: f32,
    rotation_z: f32,
    colour_r: f32,
    colour_g: f32,
    colour_b: f32,
) {
    let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
    let level = crate::entities::config::LodLevel {
        max_distance: max_distance.is_finite().then_some(max_distance),
        model: some(model),
        variant: some(variant),
        shape: crate::entities::config::MeshShape::parse(shape),
        scale: ((scale_x, scale_y, scale_z) != (1.0, 1.0, 1.0))
            .then_some([scale_x, scale_y, scale_z]),
        rotation: ((rotation_x, rotation_y, rotation_z) != (0.0, 0.0, 0.0))
            .then_some([rotation_x, rotation_y, rotation_z]),
        colour: (colour_r >= 0.0).then(|| vec![colour_r, colour_g, colour_b]),
        ..Default::default()
    };
    LADDER_DRAFT.with(|draft| draft.borrow_mut().push(level));
}

/// Apply the draft as the live ladder.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_ladder_commit() {
    let levels = LADDER_DRAFT.with(|draft| std::mem::take(&mut *draft.borrow_mut()));
    push_command(ViewerCommand::SetLadder(levels));
}

/// Re-fetch the ladder's assets and rebuild the subject from them — the
/// "I just regenerated a level, show me it" button, without a page reload.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_reload_assets() {
    push_command(ViewerCommand::ReloadAssets);
}

/// Triangles, textures, camera distance and the level on screen, as JSON.
///
/// Polled by the panel rather than pushed: the numbers change when the subject
/// changes and the distance changes when the mouse moves, and a poll keeps the
/// wasm side free of a JS callback it would otherwise have to hold.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_stats() -> String {
    stats::stats_json()
}

// ── Billboard capture bridge ─────────────────────────────────────────────────

/// Bake the far-LOD billboard atlas from the loaded subject. The result is
/// polled out with the getters below (readback is asynchronous).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_capture_billboard(views: u32, resolution: u32, pitch_deg: f32) {
    push_command(ViewerCommand::CaptureBillboard {
        views,
        resolution,
        pitch_deg,
    });
}

/// True once a baked atlas is waiting. Poll after `viewer_capture_billboard`.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_capture_ready() -> bool {
    capture::capture_ready()
}

/// The baked atlas metadata as JSON (`width`, `height`, `source`, `views`,
/// `resolution`, `pitch`, `world_w`, `world_h`).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_capture_meta() -> String {
    capture::capture_meta()
}

/// Take the baked atlas RGBA bytes (row-major, `width`×`height`×4).
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_capture_rgba() -> Vec<u8> {
    capture::capture_take_rgba()
}

/// Clear the parked atlas once the panel has saved it.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_capture_clear() {
    capture::capture_clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_args_point_at_a_real_model() {
        assert_eq!(
            ViewerArgs::default().model.as_deref(),
            Some("assets/models/alliance_cruiser.glb")
        );
    }
}
