//! The Ultralight half of a pane — **feature `ultralight`** (issue #1122).
//!
//! Everything that decides *what a pane may say and hear* lives in this
//! module's siblings, which compile and are tested without an SDK. What lives
//! here is only the part that genuinely needs one: a real
//! [`vellum_ultralight::runtime::UltralightPane`] behind the
//! [`PaneSurface`](super::surface::PaneSurface) trait, the copy from its pixel
//! buffer into a Bevy texture, and the translation of this window's mouse and
//! keyboard into page events.
//!
//! # Why the feature gate exists
//!
//! `ul-next-sys`'s build script **downloads a proprietary SDK archive at build
//! time**. Every job in `.github/workflows/ci.yml` is `ubuntu-latest`, the wasm
//! build has no use for any of this, and a plain `cargo test` must not pay a
//! hundred-megabyte download to run four thousand unit tests. So the SDK is
//! behind `--features ultralight`, which no CI job sets, and the pane logic that
//! CI *can* check is deliberately not in this file.
//!
//! # Layout is a placeholder, and says so
//!
//! Panes are tiled left to right across the window, evenly. That is enough to
//! operate one station and to see two side by side, and it is knowingly not a
//! bridge display: **issue #1123** owns full-screen bridge display profiles and
//! **#1124** owns mouse/keyboard/independent-touch routing between them. This
//! module's job is to prove a pane is a logical client, not to lay out a bridge.
//!
//! # Running it
//!
//! ```text
//! cargo build --release --features ultralight --bin phoenix-host
//! ./target/release/phoenix-host --client-dir dist --world assets/worlds/combat_test.toml \
//!     --pane Ada
//! ```
//!
//! The Ultralight shared libraries are **not** staged beside the binary by
//! cargo. [`stage_sdk`] does it, and `phoenix-host` calls it at startup; see
//! `docs/delivery-checklist.md` for the packaging half.

use bevy::asset::RenderAssetUsages;
use bevy::image::Image;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::PrimaryWindow;

use vellum_ultralight::runtime::{
    KeyEventType, Modifiers, MouseButton as UlMouseButton, PaneSpec, RuntimeOptions,
    UltralightPane, UltralightRuntime, VirtualKeyCode,
};
use vellum_ultralight::staging;

use crate::logging::{LogCat, LogFilterConfig};

use super::document::{pane_drain_script, pane_url};
use super::registry::PaneId;
use super::surface::{pump_pane, PaneSurface, PaneSurfaceError};
use super::PaneBusResource;

/// Where the operator's panes are configured from, and where they load from.
#[derive(Resource, Clone, Debug)]
pub struct PaneDisplayConfig {
    /// `host:port` of this process's own delivery server — the address the
    /// panes' documents are published at.
    pub host_addr: String,
    /// Panes to open, in left-to-right order.
    pub panes: Vec<PaneId>,
}

/// Copy the Ultralight SDK's shared libraries beside this executable and its
/// `resources/` into the working directory.
///
/// Cargo links the SDK but stages nothing, so a freshly built binary finds the
/// import library at compile time and no shared library at run time. On Windows
/// that surfaces as a process that exits with an OS error code and **no message
/// at all**, which is indistinguishable from a crash — so this runs at startup
/// and reports by name.
///
/// Returns a human-readable summary for the operator log, or the reason it could
/// not.
pub fn stage_sdk() -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot locate this executable: {e}"))?;
    let exe_dir = exe
        .parent()
        .ok_or_else(|| "this executable has no directory".to_string())?
        .to_path_buf();
    // `target/<profile>/build/ul-next-sys-<hash>/out/ul-sdk`. From
    // `target/<profile>/phoenix-host.exe` that is a sibling directory; from a
    // test binary in `target/<profile>/deps/` it is one level up. Both are
    // checked, so a test may stage the SDK beside itself the same way the
    // shipped binary does — the loader searches the *executable's* directory,
    // and for a test that is `deps/`.
    let candidates = [
        exe_dir.join("build"),
        exe_dir
            .parent()
            .map(|p| p.join("build"))
            .unwrap_or_else(|| exe_dir.join("build")),
    ];
    let missing = staging::missing_libraries(&exe_dir);
    let Some(sdk) = candidates
        .iter()
        .find_map(|dir| staging::find_sdk_root(dir))
    else {
        return if missing.is_empty() {
            Ok(format!(
                "Ultralight libraries already staged in {}",
                exe_dir.display()
            ))
        } else {
            Err(format!(
                "the Ultralight SDK is not under {} and {} {} not beside the binary — build \
                 with --features ultralight from this checkout, or stage the SDK by hand",
                candidates[0].display(),
                missing.join(", "),
                if missing.len() == 1 { "is" } else { "are" },
            ))
        };
    };
    let staged = staging::stage(&sdk, &exe_dir, &std::path::PathBuf::from(".")).map_err(|e| {
        format!(
            "cannot stage the Ultralight SDK from {}: {e}",
            sdk.display()
        )
    })?;
    Ok(format!(
        "staged {} Ultralight libraries from {} ({}); licence: {}",
        staged.libraries.len(),
        sdk.display(),
        staged
            .resources
            .map(|p| format!("resources in {}", p.display()))
            .unwrap_or_else(|| "no resources directory".to_string()),
        staging::licence_files(&sdk)
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(", "),
    ))
}

/// One pane's Ultralight view, behind the trait the frame loop drives.
pub struct UltralightPaneSurface {
    view: UltralightPane,
    /// Set once the document has reported a completed load. Sticky: `is_loading`
    /// goes false between navigations too, and a pane navigates once.
    loaded: bool,
}

impl UltralightPaneSurface {
    /// Wrap a freshly created view.
    ///
    /// Public so an integration test can drive one pane without a Bevy `App`:
    /// the automated proof that a real console page loads and answers
    /// (`tests/native_host_pane_ultralight.rs`) needs a runtime, a view and this
    /// wrapper, and nothing else this module builds.
    pub fn new(view: UltralightPane) -> Self {
        Self {
            view,
            loaded: false,
        }
    }

    /// The underlying view, for input forwarding and the pixel copy.
    pub fn view_mut(&mut self) -> &mut UltralightPane {
        &mut self.view
    }

    /// Ask the view whether its document has finished loading, and remember the
    /// answer.
    ///
    /// Sticky, deliberately: `is_loading` also goes false *between* navigations,
    /// and a pane navigates exactly once.
    pub fn refresh_loaded(&mut self) -> bool {
        if !self.loaded && !self.view.is_loading() {
            self.loaded = true;
        }
        self.loaded
    }
}

impl PaneSurface for UltralightPaneSurface {
    fn load(&mut self, url: &str) -> Result<(), PaneSurfaceError> {
        self.loaded = false;
        self.view
            .load_url(url)
            .map_err(|e| PaneSurfaceError::Load(e.to_string()))
    }

    fn is_ready(&self) -> bool {
        self.loaded
    }

    fn push(&mut self, script: &str) -> Result<(), PaneSurfaceError> {
        self.view
            .evaluate(script)
            .map(|_| ())
            .map_err(|e| PaneSurfaceError::Script(e.to_string()))
    }

    fn drain(&mut self) -> Vec<String> {
        match self.view.evaluate(&pane_drain_script()) {
            Ok(drained) => vellum_ultralight::bridge::split_records(&drained)
                .into_iter()
                .map(str::to_string)
                .collect(),
            // A throw here is the same ordinary case a failed push is: the
            // page's own scripts have not run yet, so the drain function does
            // not exist. Next frame.
            Err(_) => Vec::new(),
        }
    }
}

/// One pane on screen: its view, its texture, and where it sits.
struct PaneWindow {
    id: PaneId,
    surface: UltralightPaneSurface,
    image: Handle<Image>,
    /// Top-left corner in physical pixels, within the primary window.
    origin: (u32, u32),
    size: (u32, u32),
}

/// The Ultralight runtime and every pane hanging off it.
///
/// `!Send` by construction — `Renderer` and `View` are raw pointers with thread
/// affinity — so it is a non-send resource and only main-thread systems touch
/// it.
pub struct PaneHost {
    runtime: UltralightRuntime,
    windows: Vec<PaneWindow>,
    /// Which pane last had the pointer over it. Ultralight drops input into an
    /// unfocused view, so exactly one pane holds focus at a time.
    focused: Option<usize>,
}

/// Set once initialisation has hard-failed, so it stops retrying every frame.
#[derive(Resource)]
struct PaneHostFailed;

/// Marks the UI node showing one pane's texture.
#[derive(Component)]
struct PaneCanvas;

/// Registers the pane display systems.
///
/// Adding it without a [`PaneDisplayConfig`] and a [`PaneBus`] resource is a
/// no-op, so a host can install it unconditionally.
pub struct PaneDisplayPlugin;

impl Plugin for PaneDisplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, init_pane_host)
            .add_systems(Update, (forward_pane_input, drive_panes).chain());
    }
}

/// Create the runtime, the views and their UI nodes once the window exists.
///
/// An exclusive system that runs every frame and does its work once: it retries
/// while the primary window is not yet up, then stops on success or on a logged
/// hard failure.
fn init_pane_host(world: &mut World) {
    if world.get_non_send_resource::<PaneHost>().is_some()
        || world.get_resource::<PaneHostFailed>().is_some()
        || world.get_resource::<PaneDisplayConfig>().is_none()
    {
        return;
    }
    let Some((window_width, window_height, scale)) = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .iter(world)
        .next()
        .map(|w| {
            (
                w.physical_width().max(1),
                w.physical_height().max(1),
                w.scale_factor() as f64,
            )
        })
    else {
        return;
    };

    let config = world.resource::<PaneDisplayConfig>().clone();
    if config.panes.is_empty() {
        world.insert_resource(PaneHostFailed);
        return;
    }
    // The bus itself is read per frame by `drive_panes`; what matters here is
    // that there IS one, because a pane host with no bus would draw consoles
    // nothing could talk to.
    if world.get_resource::<PaneBusResource>().is_none() {
        world.insert_resource(PaneHostFailed);
        return;
    }

    let runtime = match UltralightRuntime::start(&RuntimeOptions::default()) {
        Ok(runtime) => runtime,
        Err(e) => {
            error!(target: LogCat::Lobby.target(), "pane host: {e}");
            world.insert_resource(PaneHostFailed);
            return;
        }
    };

    // Even left-to-right tiles. Issue #1123 owns real bridge display profiles;
    // this is enough to operate one station and to see two side by side.
    let count = config.panes.len() as u32;
    let tile_width = (window_width / count).max(1);
    let mut windows = Vec::new();
    for (index, id) in config.panes.iter().copied().enumerate() {
        let origin = (tile_width * index as u32, 0);
        let size = (tile_width, window_height);
        let spec = PaneSpec {
            width: size.0,
            height: size.1,
            device_scale: scale,
            transparent: false,
        };
        let view = match runtime.create_pane(&spec) {
            Ok(view) => view,
            Err(e) => {
                error!(target: LogCat::Lobby.target(), "pane host: {id}: {e}");
                world.insert_resource(PaneHostFailed);
                return;
            }
        };
        let mut surface = UltralightPaneSurface::new(view);
        let url = pane_url(&config.host_addr, id);
        if let Err(e) = surface.load(&url) {
            error!(target: LogCat::Lobby.target(), "pane host: {id} could not load {url}: {e}");
            world.insert_resource(PaneHostFailed);
            return;
        }
        let image = Image::new_fill(
            Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &[0, 0, 0, 255],
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        );
        let handle = world.resource_mut::<Assets<Image>>().add(image);
        world.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(origin.0 as f32 / scale as f32),
                top: Val::Px(origin.1 as f32 / scale as f32),
                width: Val::Px(size.0 as f32 / scale as f32),
                height: Val::Px(size.1 as f32 / scale as f32),
                ..default()
            },
            ImageNode::new(handle.clone()),
            PaneCanvas,
        ));
        windows.push(PaneWindow {
            id,
            surface,
            image: handle,
            origin,
            size,
        });
        info!(
            target: LogCat::Lobby.target(),
            "pane host: {id} showing {url} at {}x{}", size.0, size.1
        );
    }

    world.insert_non_send_resource(PaneHost {
        runtime,
        windows,
        focused: None,
    });
}

/// Route this window's pointer and keyboard into the pane under the cursor.
///
/// Deliberately minimal, and #1124's to replace: one pointer, one focused pane,
/// the left button, the wheel, and text. That is enough to operate a console;
/// independent per-display touch routing is that issue's whole subject.
fn forward_pane_input(
    host: Option<NonSendMut<PaneHost>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mouse: Res<ButtonInput<bevy::input::mouse::MouseButton>>,
    scroll: Res<bevy::input::mouse::AccumulatedMouseScroll>,
    mut keys: MessageReader<bevy::input::keyboard::KeyboardInput>,
) {
    let Some(mut host) = host else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let scale = window.scale_factor();

    if let Some(cursor) = window.cursor_position() {
        // Logical pixels; the view's device scale makes them equal page CSS
        // pixels, so no conversion is needed in the hot path.
        let physical = (cursor.x * scale, cursor.y * scale);
        let over = host.windows.iter().position(|w| {
            physical.0 >= w.origin.0 as f32
                && physical.0 < (w.origin.0 + w.size.0) as f32
                && physical.1 >= w.origin.1 as f32
                && physical.1 < (w.origin.1 + w.size.1) as f32
        });
        if over != host.focused {
            if let Some(previous) = host.focused {
                if let Some(w) = host.windows.get(previous) {
                    w.surface.view.unfocus();
                }
            }
            if let Some(next) = over {
                if let Some(w) = host.windows.get(next) {
                    w.surface.view.focus();
                }
            }
            host.focused = over;
        }
        if let Some(index) = over {
            let origin = host.windows[index].origin;
            let x = ((physical.0 - origin.0 as f32) / scale) as i32;
            let y = ((physical.1 - origin.1 as f32) / scale) as i32;
            let pane = &mut host.windows[index];
            // Ultralight decides what is under the pointer from the MOVE, so a
            // press with no preceding move lands on whatever was hovered last.
            pane.surface.view.mouse_move(x, y);
            if mouse.just_pressed(bevy::input::mouse::MouseButton::Left) {
                pane.surface.view.mouse_down(x, y, UlMouseButton::Left);
            }
            if mouse.just_released(bevy::input::mouse::MouseButton::Left) {
                pane.surface.view.mouse_up(x, y, UlMouseButton::Left);
            }
            if scroll.delta != Vec2::ZERO {
                pane.surface
                    .view
                    .scroll(scroll.delta.x as i32, scroll.delta.y as i32);
            }
        }
    }

    // Text goes to the focused pane, wherever the pointer has since moved. A
    // console has real form fields — a comms reply, a waypoint name — and
    // `key_char` is the event that actually puts a character into one; a raw
    // key-down alone does not.
    let focused = host.focused;
    for key in keys.read() {
        if !key.state.is_pressed() {
            continue;
        }
        let Some(index) = focused else { continue };
        let pane = &mut host.windows[index];
        match &key.logical_key {
            bevy::input::keyboard::Key::Character(text) => pane.surface.view.key_char(text),
            bevy::input::keyboard::Key::Space => pane.surface.view.key_char(" "),
            bevy::input::keyboard::Key::Backspace => pane.surface.view.key(
                KeyEventType::RawKeyDown,
                VirtualKeyCode::Back,
                0,
                Modifiers::default(),
            ),
            bevy::input::keyboard::Key::Enter => pane.surface.view.key(
                KeyEventType::RawKeyDown,
                VirtualKeyCode::Return,
                0,
                Modifiers::default(),
            ),
            bevy::input::keyboard::Key::Tab => pane.surface.view.key(
                KeyEventType::RawKeyDown,
                VirtualKeyCode::Tab,
                0,
                Modifiers::default(),
            ),
            _ => {}
        }
    }
}

/// One frame for every pane: service the library, move messages both ways,
/// rasterise, and copy what repainted into each pane's texture.
fn drive_panes(
    host: Option<NonSendMut<PaneHost>>,
    bus: Option<Res<PaneBusResource>>,
    mut images: ResMut<Assets<Image>>,
    log: Option<Res<LogFilterConfig>>,
) {
    let (Some(mut host), Some(bus)) = (host, bus) else {
        return;
    };
    host.runtime.update();

    let mut pushed_this_frame = vec![false; host.windows.len()];
    for (index, pane) in host.windows.iter_mut().enumerate() {
        // A load that has finished is what makes pushes legal. Asking the view
        // each frame (rather than trusting a callback) keeps this to one place.
        let was_loaded = pane.surface.is_ready();
        if pane.surface.refresh_loaded() && !was_loaded {
            crate::pinfo!(
                log,
                LogCat::Lobby,
                "pane host: {} finished loading its console",
                pane.id
            );
        }
        let report = pump_pane(&bus.0, pane.id, &mut pane.surface);
        pushed_this_frame[index] = report.pushed > 0;
        for refusal in &report.refusals {
            crate::pwarn!(log, LogCat::Admit, "pane host: {}: {refusal}", pane.id);
        }
    }

    host.runtime.render();

    for (index, pane) in host.windows.iter_mut().enumerate() {
        let Some(image) = images.get_mut(&pane.image) else {
            continue;
        };
        let Some(data) = image.data.as_mut() else {
            continue;
        };
        // A push we just made is trusted on its own regardless of what the
        // surface reports: a plain attribute write is real DOM state that
        // changed and Ultralight's dirty-bounds tracking does not always flag
        // it.
        match pane.surface.view.copy_frame(data, pushed_this_frame[index]) {
            Ok(_) => {}
            Err(e) => crate::pwarn!(log, LogCat::Lobby, "pane host: {}: {e}", pane.id),
        }
    }

    // A pane whose page stopped draining has lost state it cannot recover by
    // waiting. Closing it hands the lobby the same disconnect a dropped phone
    // produces, and the station flips to Backfill rather than sitting in front
    // of a page that has quietly stopped agreeing with the simulation.
    for id in bus.0.take_faulted() {
        crate::pwarn!(
            log,
            LogCat::Lobby,
            "pane host: {id} stopped draining its console and overflowed its reliable \
             backlog — closing it, so its station falls back to AI control"
        );
        bus.0.close(id);
    }
}
