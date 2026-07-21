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
mod subject;

pub use camera::OrbitCamera;
pub use lighting::LightingMode;

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
            .add_systems(Startup, (setup_camera, subject::spawn_subject).chain())
            .add_systems(
                Update,
                (
                    apply_commands,
                    subject::poll_pending_model,
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
    mut skyboxes: Query<&mut bevy::core_pipeline::Skybox>,
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
                subject_state.respawn(&mut commands);
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
