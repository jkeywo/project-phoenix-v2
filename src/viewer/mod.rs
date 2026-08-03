//! Standalone model / shader viewer (`viewer.html`, `--features viewer`).
//!
//! A dev tool for iterating on how things look without booting the lobby,
//! joining a scenario, and flying somewhere. It renders **one** subject through
//! the same shared setup the game uses — [`crate::render_setup`] for the
//! skybox, camera optics and ambient fill; [`crate::entities::glb_visual`] for
//! GLB + `.model.toml` rig composition; [`crate::entities::celestial_visual`]
//! for the star/planet WGSL materials. Nothing here reimplements the render
//! path; if it did, the tool would stop being a valid reference.
//!
//! # URL parameters
//! | param | meaning |
//! |---|---|
//! | `model` | GLB path, e.g. `assets/models/alliance_cruiser.glb` |
//! | `variant` | rig sidecar variant (`large`, `small`, `cosmetic`, `lod1`…) |
//! | `entity` | entity TOML path; renders its `[star]`, `[planet]` or `[mesh]` |
//! | `lighting` | `off` \| `ambient` \| `directional` (default `ambient`) |
//! | `gizmos` | `1` to draw rig markers, target points and the extents box |
//!
//! `model` and `entity` are mutually exclusive; `model` wins. With neither,
//! the viewer defaults to `assets/models/alliance_cruiser.glb`.

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
mod gizmos;
mod lighting;
mod lod;
mod stats;
mod subject;

pub use camera::OrbitCamera;
pub use lighting::LightingMode;
pub use lod::{LadderState, LodMode};

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
    SetLodMode(LodMode),
    /// Put the camera at a given distance from the subject — how the panel
    /// jumps to a LOD band's switch distance.
    SetCameraDistance(f32),
    /// Replace the ladder with the panel's working copy, so an edited switch
    /// distance takes effect before it is saved to the sidecar.
    SetLadder(Vec<crate::entity_config::LodLevel>),
    /// Re-fetch every asset this ladder names, then rebuild the subject from
    /// what came back.
    ReloadAssets,
}

// The panel builds a ladder one level at a time rather than handing over a
// serialised one: `serde_json` is confined to codec.rs (Key Constraint 1), and
// three exports with plain scalar arguments need no wire format at all.
thread_local! {
    static LADDER_DRAFT: RefCell<Vec<crate::entity_config::LodLevel>> =
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

// ── Entry point ────────────────────────────────────────────────────────────

/// Called by `viewer.html` on load. Builds and runs the viewer app.
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn viewer_init() {
    console_error_panic_hook::set_once();

    let args = parse_url_args();
    bevy::log::info!("viewer: {args:?}");

    App::new()
        .add_plugins(DefaultPlugins.set(bevy::window::WindowPlugin {
            primary_window: Some(bevy::window::Window {
                canvas: Some("#canvas".into()),
                fit_canvas_to_parent: true,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(ViewerPlugin { args })
        .run();
}

/// The viewer's own systems, separated from `viewer_init` so the plugin is
/// testable and buildable off-wasm.
pub struct ViewerPlugin {
    pub args: ViewerArgs,
}

impl Plugin for ViewerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.args.clone())
            // The same skybox + custom-material plugins the game registers, so
            // every shader the viewer can show is the shader the game shows.
            .add_plugins(SpaceSkyboxPlugin)
            .add_plugins(crate::entity_star::StarRenderPlugin)
            .add_plugins(crate::entity_planet::PlanetRenderPlugin)
            .init_resource::<LightingMode>()
            .init_resource::<subject::SubjectState>()
            .init_resource::<lod::LadderState>()
            .init_resource::<lod::LodMode>()
            .init_resource::<stats::SubjectStats>()
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
                ),
            );
    }
}

fn setup_camera(mut commands: Commands, skybox: Res<SpaceSkyboxAsset>) {
    commands.spawn((
        ViewerCamera,
        Camera3d::default(),
        game_camera_projection(),
        space_skybox(&skybox),
        OrbitCamera::default(),
        Transform::default(),
    ));
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
            ViewerCommand::SetLodMode(mode) => *lod_mode = mode,
            ViewerCommand::SetCameraDistance(distance) => {
                for mut orbit in &mut cameras {
                    orbit.radius = distance.max(MIN_CAMERA_DISTANCE);
                    // Distance came from the panel, so framing must not
                    // overwrite it when the next level's extents arrive.
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
                    let rel = path.strip_prefix("assets/").unwrap_or(&path).to_string();
                    asset_server.reload(rel);
                }
                // The rebuild waits for the new bytes; see
                // `respawn_on_asset_reload`. Respawning now would rebuild from
                // the very assets being replaced.
                subject_state.reloading = true;
            }
        }
        lighting.set_changed();
    }
}

// ── URL parsing ────────────────────────────────────────────────────────────

#[cfg(target_arch = "wasm32")]
fn parse_url_args() -> ViewerArgs {
    let search = web_sys::window()
        .and_then(|w| w.location().search().ok())
        .unwrap_or_default();
    let mut args = ViewerArgs {
        model: None,
        variant: None,
        entity: None,
        gizmos: false,
    };
    let mut lighting = LightingMode::default();
    for (key, value) in parse_query(&search) {
        match key.as_str() {
            "model" => args.model = Some(value),
            "variant" => args.variant = Some(value),
            "entity" => args.entity = Some(value),
            "gizmos" => args.gizmos = value == "1" || value == "true",
            "lighting" => lighting.mode = lighting::Mode::parse(&value),
            _ => bevy::log::warn!("viewer: ignoring unknown URL parameter '{key}'"),
        }
    }
    if args.model.is_none() && args.entity.is_none() {
        args.model = ViewerArgs::default().model;
    }
    push_command(ViewerCommand::SetLighting(lighting));
    args
}

/// Split a `?a=b&c=d` query string into decoded key/value pairs.
///
/// Kept as a plain function (rather than reaching for `web_sys::UrlSearchParams`)
/// so it is unit-testable off-wasm.
pub fn parse_query(search: &str) -> Vec<(String, String)> {
    search
        .trim_start_matches('?')
        .split('&')
        .filter(|p| !p.is_empty())
        .filter_map(|pair| {
            let (k, v) = pair.split_once('=')?;
            Some((percent_decode(k), percent_decode(v)))
        })
        .collect()
}

/// Minimal percent-decoding, plus `+` → space. Model and entity paths are
/// plain ASCII paths, so this only needs to handle the escaping a browser
/// applies to `/` and friends.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        i += 3;
                    }
                    // Not a valid escape — take the '%' literally.
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

// ── JS control surface ─────────────────────────────────────────────────────

// TOML fetching (rig sidecars, entity configs) reuses the server bridge's
// `set_world_fetch_callback` + `wasm_push_sidecar_toml` exports — the same pair
// `server.html` wires up. See `viewer.html` for the JS side.

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
    let level = crate::entity_config::LodLevel {
        max_distance: max_distance.is_finite().then_some(max_distance),
        model: some(model),
        variant: some(variant),
        shape: crate::entity_config::MeshShape::parse(shape),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_query() {
        let got = parse_query("?model=assets/models/x.glb&gizmos=1");
        assert_eq!(
            got,
            vec![
                ("model".to_string(), "assets/models/x.glb".to_string()),
                ("gizmos".to_string(), "1".to_string()),
            ]
        );
    }

    #[test]
    fn decodes_percent_escaped_paths() {
        let got = parse_query("?model=assets%2Fmodels%2Fx.glb");
        assert_eq!(got[0].1, "assets/models/x.glb");
    }

    #[test]
    fn empty_query_yields_nothing() {
        assert!(parse_query("").is_empty());
        assert!(parse_query("?").is_empty());
    }

    #[test]
    fn ignores_valueless_fragments() {
        let got = parse_query("?model=a.glb&junk&gizmos=1");
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn default_args_point_at_a_real_model() {
        assert_eq!(
            ViewerArgs::default().model.as_deref(),
            Some("assets/models/alliance_cruiser.glb")
        );
    }
}
