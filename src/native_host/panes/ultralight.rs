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
//! behind `--features ultralight`, and the pane logic CI *can* check is
//! deliberately not in this file.
//!
//! Default-off is not by itself enough, and assuming it was is how this feature
//! reached CI once already: `--all-features` enables everything declared, which
//! is exactly what a workspace clippy step asks for. `ci.yml`'s clippy step
//! therefore names its features explicitly. See `Cargo.toml`'s `[features]`.
//!
//! # This loop runs on the simulation's thread
//!
//! [`drive_panes`] is an `Update` system on the Bevy main thread, and every
//! `evaluate_script` it makes is synchronous. `FixedUpdate` runs `SimSet` on that
//! same thread (AGENTS.md rule 7), so page JavaScript time is *simulation* time
//! for every participant on the ship. [`pump_pane`] bounds the pushes one pane
//! may take per frame for that reason — see `super::surface`'s module note.
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
    KeyEventType, Modifiers, MouseButton as UlMouseButton, PaneSession, PaneSpec, RuntimeOptions,
    UltralightPane, UltralightRuntime, VirtualKeyCode,
};
use vellum_ultralight::staging;

use crate::logging::{LogCat, LogFilterConfig};

use super::document::pane_drain_script;
use super::recovery::{service_faults, PaneFault};
use super::registry::PaneId;
use super::surface::{pump_pane, PaneSurface, PaneSurfaceError};
use super::PaneBusResource;

/// Where the operator's panes are configured from, and where they load from.
#[derive(Resource, Clone, Debug)]
pub struct PaneDisplayConfig {
    /// Panes to open in left-to-right order, each with the URL its view
    /// navigates to.
    ///
    /// The URL is carried rather than rebuilt from an id: it holds the pane's
    /// session token and its document nonce (see `super::document`), so nothing
    /// downstream of `LocalPanes` can construct one it was not given — which is
    /// the point.
    pub panes: Vec<(PaneId, String)>,
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

/// How many consecutive frames a pane's frame copy may fail before the view is
/// treated as crashed (issue #1125).
///
/// A `copy_frame` error is ordinarily transient — a repaint mid-flight, a buffer
/// not ready — so a single one is not a crash. A view that has genuinely died
/// (a lost surface, a renderer that stopped answering) fails *every* frame, so a
/// short run of consecutive failures is the honest crash signal. Sized to about
/// half a second at 60 fps: long enough not to fire on a blip, short enough that
/// a dead console flips its station to Backfill promptly.
const VIEW_CRASH_COPY_FAILURES: u32 = 30;

/// One pane on screen: its view, its texture, and where it sits.
struct PaneWindow {
    id: PaneId,
    surface: UltralightPaneSurface,
    image: Handle<Image>,
    /// The UI node showing [`image`](Self::image), so a closed pane's canvas can
    /// be despawned rather than left on screen showing a page nothing talks to.
    canvas: Entity,
    /// Top-left corner in physical pixels, within the primary window.
    origin: (u32, u32),
    size: (u32, u32),
    /// Consecutive frames whose copy failed. Reset on any success; a run past
    /// [`VIEW_CRASH_COPY_FAILURES`] faults the pane as a crashed view (#1125).
    copy_failures: u32,
}

/// Where a named pane's view sits, kept so a pane recreated after a crash
/// (issue #1125) rebuilds in the same place — its participant name is stable
/// across the recreation, its [`PaneId`] is not.
#[derive(Clone)]
struct PaneSlotGeometry {
    name: String,
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
    /// Each named pane's on-screen slot, so a recreated pane rebuilds where its
    /// predecessor sat. Built once at init from the tiling; a name that is not
    /// here has no home and its recreated view is skipped with a warning.
    layout: Vec<PaneSlotGeometry>,
    /// The primary window's device scale, captured at init for building a
    /// recreated pane's spec and node.
    scale: f64,
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
///
/// The two `run_if`s keep that promise in an app with no *window*, which is a
/// real composition rather than a hypothetical one:
/// `NativeRenderSurface::Contract` stands up no `InputPlugin` and no image
/// assets, and `tests/native_host_panes.rs` builds exactly that with panes
/// attached. Bevy validates a system's parameters when it runs, so a bare
/// `Res<ButtonInput<_>>` there is not an inert system — it is a **panic**, in a
/// host that would otherwise be fine. `Option<Res<_>>` is the shape AGENTS.md
/// prescribes for the same reason; a run condition says it once for two systems.
pub struct PaneDisplayPlugin;

impl Plugin for PaneDisplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, init_pane_host).add_systems(
            Update,
            (
                forward_pane_input
                    .run_if(resource_exists::<ButtonInput<bevy::input::mouse::MouseButton>>),
                drive_panes.run_if(resource_exists::<Assets<Image>>),
            )
                .chain(),
        );
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
    // The `plog!` family, like `drive_panes` below: an exclusive system can
    // read `LogFilterConfig` off the world, so the bare-macro exemption
    // AGENTS.md grants to "plain helper fns with no config in scope" does not
    // apply here. Cloned once rather than held, because everything after this
    // takes `&mut World`.
    let log = world.get_resource::<LogFilterConfig>().cloned();
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
            crate::perror!(log, LogCat::Lobby, "pane host: {e}");
            world.insert_resource(PaneHostFailed);
            return;
        }
    };

    // The bus, for resolving each pane's participant name into the layout that a
    // recreated pane (issue #1125) rebuilds against.
    let bus = world.resource::<PaneBusResource>().clone();

    // Even left-to-right tiles. Issue #1123 owns real bridge display profiles;
    // this is enough to operate one station and to see two side by side.
    let count = config.panes.len() as u32;
    let tile_width = (window_width / count).max(1);
    let mut windows = Vec::new();
    let mut layout: Vec<PaneSlotGeometry> = Vec::new();
    for (index, (id, url)) in config.panes.iter().enumerate() {
        let (id, url) = (*id, url.as_str());
        let origin = (tile_width * index as u32, 0);
        let size = (tile_width, window_height);
        // Record this pane's slot by its stable participant name, so a pane
        // recreated after a view crash reopens in the same place.
        if let Some(name) = bus.0.name_of(id) {
            layout.push(PaneSlotGeometry { name, origin, size });
        }
        let spec = PaneSpec {
            width: size.0,
            height: size.1,
            device_scale: scale,
            transparent: false,
            // One storage session per pane, named after the pane and never
            // written to disk. Without it every pane lands in Ultralight's
            // single persistent default session, and since every pane document
            // is served from this host's own origin they would share one
            // cookie jar and one `localStorage` — including the
            // `session-token` key `gui/session-token.js` reads, which is the
            // one value that decides which participant a page is. The ids are
            // never reissued (`super::registry::PaneId`), so no two panes in a
            // process can collide on a name.
            session: Some(PaneSession::ephemeral(id.to_string())),
        };
        let view = match runtime.create_pane(&spec) {
            Ok(view) => view,
            Err(e) => {
                crate::perror!(log, LogCat::Lobby, "pane host: {id}: {e}");
                world.insert_resource(PaneHostFailed);
                return;
            }
        };
        let mut surface = UltralightPaneSurface::new(view);
        if let Err(e) = surface.load(url) {
            crate::perror!(
                log,
                LogCat::Lobby,
                "pane host: {id} could not load its console: {e}"
            );
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
        let canvas = world
            .spawn((
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
            ))
            .id();
        windows.push(PaneWindow {
            id,
            surface,
            image: handle,
            canvas,
            origin,
            size,
            copy_failures: 0,
        });
        // The URL is NOT logged: it carries this pane's session token in its
        // fragment, and an operator log is a file, a scrollback and a
        // screenshot. `phoenix-host` prints the first eight characters of the
        // token when it opens the pane, which is enough to correlate.
        crate::pinfo!(
            log,
            LogCat::Lobby,
            "pane host: {id} showing its console at {}x{}",
            size.0,
            size.1
        );
    }

    world.insert_non_send_resource(PaneHost {
        runtime,
        windows,
        focused: None,
        layout,
        scale,
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
///
/// Runs on the Bevy main thread, which is the simulation's — see the module
/// note, and [`pump_pane`]'s per-frame push budget.
fn drive_panes(
    host: Option<NonSendMut<PaneHost>>,
    bus: Option<Res<PaneBusResource>>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
    log: Option<Res<LogFilterConfig>>,
) {
    let (Some(mut host), Some(bus)) = (host, bus) else {
        return;
    };
    // Panes closed since the last frame — by a fault below, by the operator, or
    // by anything else holding the bus — lose their view here, BEFORE anything
    // is pumped or drawn. A view left behind is not inert: `forward_pane_input`
    // would still focus it and type into it, `pump_pane` would still drain a
    // live page's records into a registry that refuses them, once per frame,
    // for the rest of the run.
    retire_closed_panes(&mut host, &bus, &mut commands, &log);
    // Panes recreated after a fault (issue #1125) get a fresh view here, BEFORE
    // the frame drives them, so a pane brought back on the same identity reloads
    // its console and reconnects — the in-process analogue of a phone redialling.
    open_pending_views(&mut host, &bus, &mut images, &mut commands, &log);
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
            Ok(_) => pane.copy_failures = 0,
            Err(e) => {
                pane.copy_failures += 1;
                crate::pwarn!(
                    log,
                    LogCat::Lobby,
                    "pane host: {} frame copy failed ({}/{}): {e}",
                    pane.id,
                    pane.copy_failures,
                    VIEW_CRASH_COPY_FAILURES
                );
                // A view that fails EVERY frame has crashed — a lost surface, a
                // renderer that stopped answering — where a one-off failure is a
                // transient. A run past the threshold is the honest crash signal
                // (issue #1125): fault it so it rides the same close → Backfill →
                // recreate path an inbox overflow does.
                if pane.copy_failures >= VIEW_CRASH_COPY_FAILURES {
                    bus.0.fault(pane.id, PaneFault::ViewCrashed);
                }
            }
        }
    }

    // Service every faulted pane — a page that stopped draining (inbox overflow)
    // or a view that crashed (issue #1125). Each is closed, which hands the lobby
    // the same disconnect a dropped phone produces so its station flips to
    // Backfill; a crash also recreates the pane on the same identity, whose view
    // `open_pending_views` builds next frame so the human reconnects.
    //
    // `PaneBus::close` also withdraws the pane's document, so its path stops
    // resolving. The VIEW goes at the top of the next frame, through
    // `retire_closed_panes` — one path for every way a pane can be closed rather
    // than a teardown that only the fault route remembers to do.
    for outcome in service_faults(&bus.0) {
        match &outcome.recreated {
            Some((new_id, _)) => crate::pwarn!(
                log,
                LogCat::Lobby,
                "pane host: {} {} — closing it (its station falls back to AI control) and \
                 reopening it as {} on the same identity so the console reconnects",
                outcome.failed,
                outcome.fault.reason(),
                new_id
            ),
            None => crate::pwarn!(
                log,
                LogCat::Lobby,
                "pane host: {} {} — closing it, so its station falls back to AI control",
                outcome.failed,
                outcome.fault.reason()
            ),
        }
    }
}

/// Build an Ultralight view for every pane recreated after a fault (issue #1125).
///
/// A recreated pane carries the same participant identity as the one it replaces,
/// so it rebuilds in that name's stored [`PaneSlotGeometry`] — the same tile on
/// screen — and reloads the same console. A name with no stored slot (which
/// should not happen for a recreation) is skipped with a warning rather than
/// guessed at.
fn open_pending_views(
    host: &mut PaneHost,
    bus: &PaneBusResource,
    images: &mut Assets<Image>,
    commands: &mut Commands,
    log: &Option<Res<LogFilterConfig>>,
) {
    for (new_id, url) in bus.0.take_pending_views() {
        let Some(name) = bus.0.name_of(new_id) else {
            continue;
        };
        let Some(slot) = host.layout.iter().find(|s| s.name == name).cloned() else {
            crate::pwarn!(
                log,
                LogCat::Lobby,
                "pane host: recreated {new_id} ({name}) has no stored layout slot; its view is \
                 not rebuilt — its station stays on AI control"
            );
            continue;
        };
        match make_pane_view(
            &host.runtime,
            images,
            commands,
            new_id,
            &url,
            slot.origin,
            slot.size,
            host.scale,
        ) {
            Ok(window) => {
                host.windows.push(window);
                crate::pinfo!(
                    log,
                    LogCat::Lobby,
                    "pane host: {new_id} ({name}) recreated after a fault — reloading its \
                     console to reconnect on the same identity"
                );
            }
            Err(e) => crate::pwarn!(
                log,
                LogCat::Lobby,
                "pane host: could not rebuild the view for {new_id} ({name}): {e}"
            ),
        }
    }
}

/// Create one pane's Ultralight view, its texture and its on-screen canvas, at
/// `origin`/`size` (issue #1125's recreation path).
///
/// The same construction `init_pane_host` does inline, but against
/// `Assets<Image>`/`Commands` rather than an exclusive `&mut World`, because
/// `drive_panes` is an ordinary system. A creation or load failure here fails
/// only THIS pane — its station simply stays on Backfill — rather than the whole
/// host, which is right for a recovery path.
fn make_pane_view(
    runtime: &UltralightRuntime,
    images: &mut Assets<Image>,
    commands: &mut Commands,
    id: PaneId,
    url: &str,
    origin: (u32, u32),
    size: (u32, u32),
    scale: f64,
) -> Result<PaneWindow, PaneSurfaceError> {
    let spec = PaneSpec {
        width: size.0,
        height: size.1,
        device_scale: scale,
        transparent: false,
        session: Some(PaneSession::ephemeral(id.to_string())),
    };
    let view = runtime
        .create_pane(&spec)
        .map_err(|e| PaneSurfaceError::Load(e.to_string()))?;
    let mut surface = UltralightPaneSurface::new(view);
    surface.load(url)?;
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
    let handle = images.add(image);
    let canvas = commands
        .spawn((
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
        ))
        .id();
    Ok(PaneWindow {
        id,
        surface,
        image: handle,
        canvas,
        origin,
        size,
        copy_failures: 0,
    })
}

/// Tear down the view of every pane the bus no longer lists as open.
///
/// A closed pane's station is held and flipped to `Backfill`, exactly as a
/// dropped phone's is, and the mission carries on with AI at that console.
///
/// **Recreation is a separate step, and a separate identity question** (issue
/// #1125). This only tears down; [`open_pending_views`] builds a *new* view for a
/// pane the bus recreated after a crash. That recreated pane keeps the same
/// participant *token* (so its page's `Identify` is a reconnect the lobby answers
/// by restoring the held station) but gets a new `PaneId` — ids are never
/// reissued (`super::registry::PaneRegistry`) — so the two steps never collide:
/// this retires the old handle's view, `open_pending_views` opens the new one's.
///
/// Each closure is logged **once**, because the window is gone afterwards.
fn retire_closed_panes(
    host: &mut PaneHost,
    bus: &PaneBusResource,
    commands: &mut Commands,
    log: &Option<Res<LogFilterConfig>>,
) {
    if host.windows.is_empty() {
        return;
    }
    let open = bus.0.open_pane_ids();
    if host.windows.iter().all(|w| open.contains(&w.id)) {
        return;
    }
    host.windows.retain(|window| {
        if open.contains(&window.id) {
            return true;
        }
        commands.entity(window.canvas).despawn();
        crate::pinfo!(
            log,
            LogCat::Lobby,
            "pane host: {} is closed — its view is torn down and its station is \
             the lobby's business now",
            window.id
        );
        false
    });
    // Every remaining index has shifted, and focus is an index. Dropping it
    // rather than trying to translate it is right: `forward_pane_input` takes
    // focus again the moment the pointer is over a pane.
    host.focused = None;
}
