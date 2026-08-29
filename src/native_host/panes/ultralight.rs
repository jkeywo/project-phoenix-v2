//! The Ultralight half of a pane — **feature `ultralight`** (issues #1122,
//! #1124).
//!
//! Everything that decides *what a pane may say and hear* lives in this
//! module's siblings, which compile and are tested without an SDK. What lives
//! here is only the part that genuinely needs one: a real
//! [`vellum_ultralight::runtime::UltralightPane`] behind the
//! [`PaneSurface`](super::surface::PaneSurface) trait, the copy from its pixel
//! buffer into a Bevy texture, and the translation of this machine's mouse,
//! keyboard and touch into page events.
//!
//! The **routing decisions** — which pane a physical coordinate belongs to,
//! which pane holds keyboard focus, which pane a touch contact is captured by —
//! are the pure [`crate::native_host::input_routing`] model, tested in CI with no
//! hardware. This file is the thin winit/Bevy layer that reads Bevy's
//! `CursorMoved`/`ButtonInput`/`KeyboardInput`/`TouchInput` per window, asks that
//! model where the event goes, and injects it into the target view.
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
//! # Layout: single-window tiling, or composited onto Station windows
//!
//! With no bridge profile the panes tile left-to-right across the process's
//! primary (viewscreen) window — enough to operate one station and to see two
//! side by side. With a `--profile` (issue #1123) each pane is instead
//! **composited onto its assigned Station window**: a per-Station 2-D camera
//! renders the pane's texture on that window, and input from that physical window
//! is routed to it. That is the pane→Station-window compositing #1123 deferred to
//! this issue. Panes the profile does not name a Station slot for fall back to
//! tiling on the primary window, so a mixed launch never leaves a pane with
//! nowhere to draw. See [`init_pane_host`].
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
use bevy::camera::{ClearColorConfig, RenderTarget};
use bevy::core_pipeline::core_2d::graph::Core2d;
use bevy::image::Image;
use bevy::input::keyboard::{Key, KeyCode, KeyboardInput};
use bevy::input::mouse::{AccumulatedMouseScroll, MouseButton};
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::input::ButtonInput;
use bevy::prelude::*;
use bevy::render::camera::CameraRenderGraph;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy::window::{PrimaryWindow, WindowRef};

use vellum_ultralight::runtime::{
    KeyEventType, Modifiers, MouseButton as UlMouseButton, PaneSession, PaneSpec, RuntimeOptions,
    UltralightPane, UltralightRuntime, VirtualKeyCode,
};
use vellum_ultralight::staging;

use crate::logging::{LogCat, LogFilterConfig};
use crate::native_host::bridge_display::BridgeStationSurfaces;
use crate::native_host::bridge_profile::PaneRect;
use crate::native_host::input_routing::{
    pointer_follow_focus, ContactCaptureMap, FocusRing, MouseCapture, PaneHit, PanePlacement,
    PaneRouter, PointerMotion, WindowKey,
};

use super::document::pane_drain_script;
use super::recovery::{service_faults, PaneFault};
use super::registry::PaneId;
use super::surface::{pump_pane, PaneSurface, PaneSurfaceError};
use super::PaneBusResource;

// ── focus-indicator geometry (issue #1124, acceptance criterion 2) ───────────
//
// Not gameplay values and not designer tunables: these are the dimensions of the
// keyboard-focus indicator, an accessibility affordance, expressed in a pane's
// own LOGICAL pixels so they read the same at any monitor scale. The indicator is
// a full ring PLUS four corner brackets, and it is drawn on exactly the focused
// pane and on no other — so focus is shown by the PRESENCE of that shape, not by
// a colour change. That is what makes it independent of colour (WCAG 1.4.1): a
// colour-blind operator, or one reading a washed-out bridge display, sees a
// bracketed frame appear, not one border changing hue.

/// The thickness of the focus ring and its corner brackets, logical pixels.
const FOCUS_RING_THICKNESS_PX: f32 = 3.0;
/// The arm length of each corner bracket, logical pixels — long enough to read as
/// a deliberate reticle rather than a rounded corner.
const FOCUS_RING_BRACKET_PX: f32 = 26.0;
/// How far the ring is inset from the pane edge, logical pixels, so the frame
/// sits just inside the console rather than being clipped at the window edge.
const FOCUS_RING_INSET_PX: f32 = 2.0;

/// The focus indicator's colour. A colour is still needed to draw it; the
/// accessibility guarantee is that focus is conveyed by the ring's PRESENCE and
/// its bracket shape, not by this value — an unfocused pane has no ring at all,
/// so no colour discrimination is required to tell focused from unfocused. A
/// near-opaque white reads on the dark console chrome the same way the viewscreen
/// HUD text does (`server::renderer`).
fn focus_ring_color() -> Color {
    Color::srgba(1.0, 1.0, 1.0, 0.92)
}

/// Marks an entity that is part of the keyboard-focus indicator, so the whole
/// reticle can be despawned as one when focus moves.
#[derive(Component)]
struct FocusRingNode;

/// Where the operator's panes are configured from, and where they load from.
#[derive(Resource, Clone, Debug)]
pub struct PaneDisplayConfig {
    /// Panes to open, each with the URL its view navigates to and the
    /// participant label it was opened under.
    ///
    /// The URL carries the pane's session token and its document nonce (see
    /// `super::document`), so nothing downstream of `LocalPanes` can construct one
    /// it was not given. The label is the `--pane <NAME>` name, and is what ties a
    /// pane to a bridge profile's Station pane slot of the same label (issue
    /// #1124) — kept out of the URL, because it decides layout, not identity.
    pub panes: Vec<PaneDisplayEntry>,
}

/// One configured pane: its handle, the URL its view loads, and the participant
/// label that matches it to a Station slot.
#[derive(Clone, Debug)]
pub struct PaneDisplayEntry {
    pub id: PaneId,
    pub url: String,
    pub label: String,
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

/// One pane on screen: its view, its texture, where it sits, and which window it
/// is composited on.
struct PaneWindow {
    id: PaneId,
    surface: UltralightPaneSurface,
    image: Handle<Image>,
    /// The UI node showing [`image`](Self::image), so a closed pane's canvas can
    /// be despawned rather than left on screen showing a page nothing talks to.
    canvas: Entity,
    /// The OS window this pane renders on and receives input from — the primary
    /// (viewscreen) window for the tiled single-window host, or a Station window
    /// for a composited one.
    window: Entity,
    /// The 2-D camera that renders this pane's canvas onto its window, when that
    /// is a Station window. `None` for a pane on the primary window, which uses
    /// the game's own default UI camera.
    station_camera: Option<Entity>,
    /// Top-left corner in physical pixels, within [`window`](Self::window).
    origin: (u32, u32),
    size: (u32, u32),
    /// This window's scale factor — the divisor from physical to page logical.
    scale: f64,
    /// This window's top-left on the virtual desktop, physical pixels; `(0, 0)`
    /// for the primary window.
    window_origin: (i32, i32),
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
    /// Each named pane's on-screen slot, so a recreated pane rebuilds where its
    /// predecessor sat. Built once at init from the tiling; a name that is not
    /// here has no home and its recreated view is skipped with a warning.
    layout: Vec<PaneSlotGeometry>,
    /// The primary (viewscreen) window, where a pane recreated after a crash
    /// (issue #1125) is rebuilt — tiled, on the game's default UI camera.
    primary_window: Entity,
    /// The primary window's device scale, captured at init for building a
    /// recreated pane's spec and node.
    scale: f64,
    /// The keyboard-focus order and which pane holds focus. Owns the model; the
    /// Ultralight `focus()`/`unfocus()` calls follow it.
    focus: FocusRing,
    /// Touch contacts pinned to the pane each began on (acceptance criterion 4).
    contacts: ContactCaptureMap,
    /// The left mouse button's capture — the pane a held drag began on, so its
    /// move and release route there wherever the cursor drifts (acceptance
    /// criterion 4, the mouse mirror of [`contacts`](Self::contacts)).
    mouse_capture: MouseCapture,
    /// Where the pointer was last seen per window, so focus follows genuine
    /// pointer motion and not the mere presence of a resting cursor (acceptance
    /// criterion 2).
    pointer_motion: PointerMotion,
    /// The spatial map from a physical coordinate to the pane under it. Rebuilt
    /// whenever a pane opens or closes.
    router: PaneRouter,
    /// The focus indicator entity currently on screen and the pane it frames, so
    /// it can be moved when focus moves and despawned when focus clears.
    ring: Option<(PaneId, Entity)>,
    /// One 2-D camera per Station window, `(window, camera)` — spawned once and
    /// reused as panes are composited onto that window.
    station_cameras: Vec<(Entity, Entity)>,
}

impl PaneHost {
    /// The current input router — its pane placements, the focus order, and the
    /// coordinate resolution. Public for the ignored integration test
    /// (`tests/native_host_input.rs`) and for diagnostics; there is no other
    /// reader, because the routing systems hold a `NonSendMut` to the whole host.
    pub fn router(&self) -> &PaneRouter {
        &self.router
    }

    /// The pane currently holding keyboard focus, if any — the observable half of
    /// [`traverse_focus_keys`] and pointer-follow focus.
    pub fn focused_pane(&self) -> Option<PaneId> {
        self.focus.focused()
    }

    fn index_of(&self, pane: PaneId) -> Option<usize> {
        self.windows.iter().position(|w| w.id == pane)
    }

    /// Give a pane's view keyboard focus and take it from whatever held it,
    /// keeping Ultralight's single-focused-view invariant in step with the model.
    fn focus_view(&self, previous: Option<PaneId>, next: Option<PaneId>) {
        if previous == next {
            return;
        }
        if let Some(prev) = previous.and_then(|p| self.index_of(p)) {
            self.windows[prev].surface.view.unfocus();
        }
        if let Some(next) = next.and_then(|p| self.index_of(p)) {
            self.windows[next].surface.view.focus();
        }
    }

    /// Rebuild the router and reconcile the focus order and touch captures after
    /// the set of open panes changed.
    fn rebuild_layout(&mut self) {
        self.router = build_router(&self.windows);
        self.focus.sync_order(self.router.focus_order());
    }
}

/// Build the pure router from the live pane windows.
fn build_router(windows: &[PaneWindow]) -> PaneRouter {
    PaneRouter::new(
        windows
            .iter()
            .map(|w| PanePlacement {
                pane: w.id,
                window: WindowKey(w.window.to_bits()),
                window_origin_x: w.window_origin.0,
                window_origin_y: w.window_origin.1,
                rect: PaneRect {
                    x: w.origin.0,
                    y: w.origin.1,
                    width: w.size.0,
                    height: w.size.1,
                },
                scale_factor: w.scale,
            })
            .collect(),
    )
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
/// The `run_if`s keep that promise in an app with no *window*, which is a real
/// composition rather than a hypothetical one: `NativeRenderSurface::Contract`
/// stands up no `InputPlugin` and no image assets, and `tests/native_host_panes.rs`
/// builds exactly that with panes attached. Bevy validates a system's parameters
/// when it runs, so a bare `Res<ButtonInput<_>>` there is not an inert system —
/// it is a **panic**, in a host that would otherwise be fine. Gating the whole
/// input group on the input plugin's own `ButtonInput<MouseButton>` says it once.
pub struct PaneDisplayPlugin;

impl Plugin for PaneDisplayPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, init_pane_host).add_systems(
            Update,
            (
                (
                    route_pointer_input,
                    route_touch_input,
                    traverse_focus_keys,
                    forward_keyboard_text,
                    update_focus_indicator,
                )
                    .chain()
                    .run_if(resource_exists::<ButtonInput<MouseButton>>),
                drive_panes.run_if(resource_exists::<Assets<Image>>),
            )
                .chain(),
        );
    }
}

/// Create the runtime, the views and their UI nodes once the windows exist.
///
/// An exclusive system that runs every frame and does its work once: it retries
/// while the primary window is not yet up (and, when a bridge profile is in
/// force, while that profile has not yet opened its Station windows), then stops
/// on success or on a logged hard failure.
fn init_pane_host(world: &mut World) {
    if world.get_non_send_resource::<PaneHost>().is_some()
        || world.get_resource::<PaneHostFailed>().is_some()
        || world.get_resource::<PaneDisplayConfig>().is_none()
    {
        return;
    }
    // With a bridge profile in force, panes are composited onto the Station
    // windows it opens — so wait until it has applied. `BridgeDisplayApplied` is
    // inserted by `apply_bridge_profile` the frame it opens the windows and
    // publishes `BridgeStationSurfaces`; before then there is nothing to
    // composite onto, and tiling on the primary window now would have to be
    // undone. A profile that names no present Station still applies, so this is a
    // bounded wait, not a hang.
    let has_profile = world
        .get_resource::<crate::native_host::bridge_display::BridgeDisplayConfig>()
        .is_some();
    if has_profile
        && world
            .get_resource::<crate::native_host::bridge_display::BridgeDisplayApplied>()
            .is_none()
    {
        return;
    }

    // The `plog!` family, like `drive_panes` below: an exclusive system can
    // read `LogFilterConfig` off the world, so the bare-macro exemption
    // AGENTS.md grants to "plain helper fns with no config in scope" does not
    // apply here. Cloned once rather than held, because everything after this
    // takes `&mut World`.
    let log = world.get_resource::<LogFilterConfig>().cloned();

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

    // The primary (viewscreen) window: the tiling fallback's home, and needed
    // even in a profile launch for any pane the profile does not seat.
    let Some((primary_entity, primary_width, primary_height, primary_scale)) = world
        .query_filtered::<(Entity, &Window), With<PrimaryWindow>>()
        .iter(world)
        .next()
        .map(|(e, w)| {
            (
                e,
                w.physical_width().max(1),
                w.physical_height().max(1),
                w.scale_factor() as f64,
            )
        })
    else {
        return;
    };

    // Where each configured pane will be composited: on a Station window if the
    // profile names a slot with its label, otherwise tiled on the primary window.
    let stations = world
        .get_resource::<BridgeStationSurfaces>()
        .cloned()
        .unwrap_or_default();
    struct Seat {
        entry: PaneDisplayEntry,
        window: Entity,
        origin: (u32, u32),
        size: (u32, u32),
        scale: f64,
        window_origin: (i32, i32),
        station: bool,
    }
    let mut seats: Vec<Seat> = Vec::new();
    let mut tiled: Vec<PaneDisplayEntry> = Vec::new();
    for entry in &config.panes {
        let seat = stations.0.iter().find_map(|surface| {
            surface
                .panes
                .iter()
                .find(|slot| slot.label == entry.label)
                .map(|slot| Seat {
                    entry: entry.clone(),
                    window: surface.window,
                    origin: (slot.rect.x, slot.rect.y),
                    size: (slot.rect.width.max(1), slot.rect.height.max(1)),
                    scale: surface.geometry.scale_factor.max(0.1),
                    window_origin: (surface.geometry.position_x, surface.geometry.position_y),
                    station: true,
                })
        });
        match seat {
            Some(seat) => seats.push(seat),
            None => tiled.push(entry.clone()),
        }
    }
    // The panes with no Station slot tile left-to-right across the primary
    // window, exactly as the pre-#1123 host laid them out.
    if !tiled.is_empty() {
        let count = tiled.len() as u32;
        let tile_width = (primary_width / count).max(1);
        for (index, entry) in tiled.into_iter().enumerate() {
            seats.push(Seat {
                entry,
                window: primary_entity,
                origin: (tile_width * index as u32, 0),
                size: (tile_width, primary_height),
                scale: primary_scale,
                window_origin: (0, 0),
                station: false,
            });
        }
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

    let mut windows = Vec::new();
    let mut station_cameras: Vec<(Entity, Entity)> = Vec::new();
    let mut layout: Vec<PaneSlotGeometry> = Vec::new();
    for seat in seats {
        let id = seat.entry.id;
        let url = seat.entry.url.as_str();
        // Record this pane's slot by its stable participant name, so a pane
        // recreated after a view crash reopens in the same place (issue #1125).
        if let Some(name) = bus.0.name_of(id) {
            layout.push(PaneSlotGeometry {
                name,
                origin: seat.origin,
                size: seat.size,
            });
        }
        // One 2-D camera per Station window, rendering that window's panes. The
        // primary window already has the game's default UI camera, so a tiled
        // pane needs none.
        let station_camera = if seat.station {
            Some(
                match station_cameras.iter().find(|(w, _)| *w == seat.window) {
                    Some((_, cam)) => *cam,
                    None => {
                        let cam = world
                            .spawn((
                                Camera2d,
                                Camera {
                                    // Render after (on top of) any default-order
                                    // camera on this window; the Station window has
                                    // none of its own, so order is not contentious.
                                    order: 0,
                                    clear_color: ClearColorConfig::Custom(Color::BLACK),
                                    ..default()
                                },
                                // In Bevy 0.18 the render target is its own component
                                // (`Camera` `#[require]`s it), not a `Camera` field:
                                // this Station camera renders onto its Station window.
                                RenderTarget::Window(WindowRef::Entity(seat.window)),
                                CameraRenderGraph::new(Core2d),
                            ))
                            .id();
                        station_cameras.push((seat.window, cam));
                        cam
                    }
                },
            )
        } else {
            None
        };

        let spec = PaneSpec {
            width: seat.size.0,
            height: seat.size.1,
            device_scale: seat.scale,
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
                width: seat.size.0,
                height: seat.size.1,
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
                    left: Val::Px(seat.origin.0 as f32 / seat.scale as f32),
                    top: Val::Px(seat.origin.1 as f32 / seat.scale as f32),
                    width: Val::Px(seat.size.0 as f32 / seat.scale as f32),
                    height: Val::Px(seat.size.1 as f32 / seat.scale as f32),
                    ..default()
                },
                ImageNode::new(handle.clone()),
                PaneCanvas,
            ))
            .id();
        // A composited pane's canvas renders on its Station camera; a tiled one
        // uses the default UI camera and takes no marker.
        if let Some(cam) = station_camera {
            world.entity_mut(canvas).insert(UiTargetCamera(cam));
        }
        windows.push(PaneWindow {
            id,
            surface,
            image: handle,
            canvas,
            window: seat.window,
            station_camera,
            origin: seat.origin,
            size: seat.size,
            scale: seat.scale,
            window_origin: seat.window_origin,
            copy_failures: 0,
        });
        // The URL is NOT logged: it carries this pane's session token in its
        // fragment, and an operator log is a file, a scrollback and a
        // screenshot. `phoenix-host` prints the first eight characters of the
        // token when it opens the pane, which is enough to correlate.
        crate::pinfo!(
            log,
            LogCat::Lobby,
            "pane host: {id} showing its console at {}x{} on {} ({})",
            seat.size.0,
            seat.size.1,
            if seat.station {
                "its Station window"
            } else {
                "the viewscreen window"
            },
            seat.entry.label,
        );
    }

    let router = build_router(&windows);
    // Seed focus onto the first pane so a pure-keyboard operator sees the reticle
    // and has a defined keyboard target the instant panes exist, rather than a
    // blank ring in which keys go nowhere until the first Ctrl+Tab (issue #1124,
    // acceptance criterion 2). The Ultralight view is told to focus to match, so
    // the first keystroke lands without a preceding click or Ctrl+Tab —
    // Ultralight drops input into an unfocused view.
    let focus = FocusRing::focused_on_first(router.focus_order());
    if let Some(first) = focus.focused() {
        if let Some(window) = windows.iter().find(|w| w.id == first) {
            window.surface.view.focus();
        }
    }
    world.insert_non_send_resource(PaneHost {
        runtime,
        windows,
        layout,
        primary_window: primary_entity,
        scale: primary_scale,
        focus,
        contacts: ContactCaptureMap::new(),
        mouse_capture: MouseCapture::new(),
        pointer_motion: PointerMotion::new(),
        router,
        ring: None,
        station_cameras,
    });
}

/// Route this frame's pointer into the pane it belongs to, and let focus follow
/// it on genuine motion.
///
/// One pointer, every window: the OS moves the cursor across the extended
/// desktop, and exactly one window reports a `cursor_position` at a time. The
/// pure router says which pane on that window the physical point lands in, and
/// the same pane takes the buttons and the wheel — one mouse operating every
/// configured surface with no mode switch (acceptance criterion 1).
///
/// Two rules from the pure model make this predictable:
///
/// * **Focus follows the pointer only on motion** (acceptance criterion 2, via
///   [`pointer_follow_focus`]). A resting cursor reports the same position every
///   frame; re-asserting focus on it would revert a Ctrl+Tab selection the next
///   frame. So focus moves only when the pointer actually moved.
/// * **The left button is captured to the pane it went down on** (acceptance
///   criterion 4, via [`MouseCapture`], mirroring the touch contact capture). A
///   drag that leaves the pane still delivers its moves and its release THERE —
///   projected past the pane edge with `project_into_pane` when the cursor has
///   drifted off it — so a cross-pane drag never sends a down to one pane and an
///   unmatched up to another.
fn route_pointer_input(
    host: Option<NonSendMut<PaneHost>>,
    windows: Query<(Entity, &Window)>,
    mouse: Res<ButtonInput<MouseButton>>,
    scroll: Res<AccumulatedMouseScroll>,
) {
    let Some(mut host) = host else {
        return;
    };
    // The window the cursor is in, its physical position there, and the pane it
    // is over (None over a gap between panes). Exactly one window reports the
    // cursor at a time on an extended desktop.
    let mut cursor: Option<(WindowKey, f64, f64, Option<PaneHit>)> = None;
    for (entity, window) in windows.iter() {
        let Some(pos) = window.cursor_position() else {
            continue;
        };
        let scale = window.scale_factor() as f64;
        // Logical → physical, then the router divides back to page logical: the
        // round trip keeps the transform in one place (the pure model) and
        // matches how a pane's device scale is set.
        let phys = (pos.x as f64 * scale, pos.y as f64 * scale);
        let key = WindowKey(entity.to_bits());
        let hit = host.router.resolve_in_window(key, phys.0, phys.1);
        cursor = Some((key, phys.0, phys.1, hit));
        break;
    }

    // Focus follows the pointer, but only on genuine motion into a pane — never
    // on the mere presence of a resting cursor, which would revert a Ctrl+Tab
    // selection every frame.
    if let Some((key, x, y, Some(hit))) = cursor {
        let previous = host.focus.focused();
        let host = &mut *host;
        if pointer_follow_focus(&mut host.focus, &mut host.pointer_motion, key, x, y, hit.pane) {
            host.focus_view(previous, Some(hit.pane));
        }
    }

    // Press: capture the left button to the pane it went down on. Ultralight
    // decides what is under the pointer from the MOVE, so a press is preceded by
    // a move in the same frame.
    if mouse.just_pressed(MouseButton::Left) {
        if let Some((_, _, _, Some(hit))) = cursor {
            if let Some(index) = host.index_of(hit.pane) {
                let view = &mut host.windows[index].surface.view;
                view.mouse_move(hit.local_x, hit.local_y);
                view.mouse_down(hit.local_x, hit.local_y, UlMouseButton::Left);
            }
            host.mouse_capture.press(hit.pane);
        }
    }

    // Move: while the button is captured, the move goes to the CAPTURED pane
    // wherever the cursor has drifted (projected past the pane edge as needed);
    // otherwise it is an ordinary hover over the pane under the cursor.
    if let Some(captured) = host.mouse_capture.captured() {
        if let Some((key, x, y, _)) = cursor {
            // The captured pane may be on a different window than the cursor now
            // sits in (a drag onto another monitor); project only when the cursor
            // is still in the pane's own window, where a within-window drag past
            // the edge resolves correctly.
            if host.router.placement(captured).map(|p| p.window) == Some(key) {
                if let Some((lx, ly)) = host.router.project_into_pane(captured, x, y) {
                    if let Some(index) = host.index_of(captured) {
                        host.windows[index].surface.view.mouse_move(lx, ly);
                    }
                }
            }
        }
    } else if let Some((_, _, _, Some(hit))) = cursor {
        if let Some(index) = host.index_of(hit.pane) {
            host.windows[index]
                .surface
                .view
                .mouse_move(hit.local_x, hit.local_y);
        }
    }

    // Release: the captured pane gets the mouse_up, and the capture is released
    // no matter where the cursor is. The up is placed at the drift position when
    // the cursor is still in the pane's window, else at the pane's own origin —
    // an in-pane coordinate that ends the press cleanly.
    if mouse.just_released(MouseButton::Left) {
        if let Some(captured) = host.mouse_capture.release() {
            let (lx, ly) = cursor
                .filter(|(key, ..)| {
                    host.router.placement(captured).map(|p| p.window) == Some(*key)
                })
                .and_then(|(_, x, y, _)| host.router.project_into_pane(captured, x, y))
                .unwrap_or((0, 0));
            if let Some(index) = host.index_of(captured) {
                host.windows[index]
                    .surface
                    .view
                    .mouse_up(lx, ly, UlMouseButton::Left);
            }
        }
    }

    // Scroll goes to the pane under the cursor.
    if scroll.delta != Vec2::ZERO {
        if let Some((_, _, _, Some(hit))) = cursor {
            if let Some(index) = host.index_of(hit.pane) {
                host.windows[index]
                    .surface
                    .view
                    .scroll(scroll.delta.x as i32, scroll.delta.y as i32);
            }
        }
    }
}

/// Route touch contacts to panes, each captured by the pane it started on
/// (acceptance criteria 3 and 4).
///
/// winit reports a `TouchInput` against the window it happened on, carrying a
/// stable contact id for the finger's whole life (down → moves → up). We treat
/// its `position` as window logical pixels, the same space as
/// `Window::cursor_position`, and convert to physical for the router. On
/// **Started** the pane is resolved and the contact PINNED to it; every
/// **Moved** and the final **Ended**/**Canceled** are routed to that pinned pane
/// no matter where the finger has drifted, so a drag stays with the console it
/// began on. Contacts on different screens carry different ids and never
/// interact.
///
/// Ultralight has no touch surface, so each contact is expressed as pointer
/// events on its pinned pane's view: a down, moves while held (a drag), and an
/// up. Two contacts on the *same* pane share that one pointer — the honest limit
/// of this mapping, disclosed in the acceptance kit; two on *different* panes are
/// genuinely independent.
///
/// # What this routes by, and what it does NOT consult
///
/// Touch routing here assumes **one borderless-fullscreen Station window per
/// touch display**: a contact is routed purely by the window winit reports it
/// against (`touch.window`), resolved among that window's panes with
/// [`PaneRouter::resolve_in_window`]. It does **not** consult the profile's
/// `[[touch]]` [`TouchMapping`](crate::native_host::bridge_profile::TouchMapping)
/// tables, and it does not use [`PaneRouter::resolve_desktop`]. `[[touch]]` is
/// recorded for issue #1123's persistence (which physical touchscreen drives
/// which display, so a setup survives a reboot) and is not read by #1124's
/// router; `resolve_desktop` exists for the future case this defers — a touch
/// panel *decoupled* from its monitor, whose device-global coordinates would be
/// mapped to a display by `[[touch]]` and then to a pane. Wiring that for real
/// decoupled panels is a future item; with the one-window-per-display assumption
/// above, winit already delivers the contact against the right window and no
/// mapping is needed. This is honest about a HITL-parked path (Part B of
/// `docs/acceptance/1124-input.md`), not a silent gap.
fn route_touch_input(
    host: Option<NonSendMut<PaneHost>>,
    windows: Query<(Entity, &Window)>,
    mut touches: MessageReader<TouchInput>,
) {
    let Some(mut host) = host else {
        return;
    };
    for touch in touches.read() {
        let Some((_, window)) = windows.iter().find(|(e, _)| *e == touch.window) else {
            continue;
        };
        let scale = window.scale_factor() as f64;
        let phys = (
            touch.position.x as f64 * scale,
            touch.position.y as f64 * scale,
        );
        let window_key = WindowKey(touch.window.to_bits());
        match touch.phase {
            TouchPhase::Started => {
                if let Some(hit) = host.router.resolve_in_window(window_key, phys.0, phys.1) {
                    if host.contacts.start(touch.id, hit.pane) {
                        // A tap gives its pane keyboard focus, exactly as a click
                        // does — so an on-screen keyboard or a physical one types
                        // into the console the operator just touched.
                        let previous = host.focus.focused();
                        if host.focus.focus(hit.pane) {
                            host.focus_view(previous, Some(hit.pane));
                        }
                        if let Some(index) = host.index_of(hit.pane) {
                            let view = &mut host.windows[index].surface.view;
                            view.mouse_move(hit.local_x, hit.local_y);
                            view.mouse_down(hit.local_x, hit.local_y, UlMouseButton::Left);
                        }
                    }
                }
            }
            TouchPhase::Moved => {
                if let Some(pane) = host.contacts.pane_for(touch.id) {
                    if let Some((lx, ly)) = host.router.project_into_pane(pane, phys.0, phys.1) {
                        if let Some(index) = host.index_of(pane) {
                            host.windows[index].surface.view.mouse_move(lx, ly);
                        }
                    }
                }
            }
            TouchPhase::Ended | TouchPhase::Canceled => {
                if let Some(pane) = host.contacts.end(touch.id) {
                    if let Some((lx, ly)) = host.router.project_into_pane(pane, phys.0, phys.1) {
                        if let Some(index) = host.index_of(pane) {
                            host.windows[index]
                                .surface
                                .view
                                .mouse_up(lx, ly, UlMouseButton::Left);
                        }
                    }
                }
            }
        }
    }
}

/// Cycle keyboard focus between panes with Ctrl+Tab / Ctrl+Shift+Tab (acceptance
/// criterion 2).
///
/// **Not plain Tab**, deliberately: a console has real form fields, and plain Tab
/// must stay the page's own field-to-field traversal. Ctrl+Tab is the
/// long-standing convention for moving between panes or tabs and no console page
/// binds it, so inter-pane focus and in-pane focus never fight over a key. The
/// model cycles; [`update_focus_indicator`] draws the visible non-colour ring on
/// wherever focus lands.
fn traverse_focus_keys(host: Option<NonSendMut<PaneHost>>, keys: Res<ButtonInput<KeyCode>>) {
    let Some(mut host) = host else {
        return;
    };
    if !keys.just_pressed(KeyCode::Tab) {
        return;
    }
    if !(keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight)) {
        return;
    }
    let backward = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
    let previous = host.focus.focused();
    let next = if backward {
        host.focus.focus_prev()
    } else {
        host.focus.focus_next()
    };
    host.focus_view(previous, next);
}

/// Deliver typed text and caret/navigation keys to the focused pane, wherever
/// the pointer has since moved.
///
/// A console has real form fields — a comms reply, a waypoint name — so the
/// operator must be able to type into one, move the caret within it and delete in
/// either direction. `key_char` is the event that actually puts a character into
/// a field; a raw key-down alone does not, which is why text is forwarded as
/// `key_char` and the editing keys (Backspace, Delete, the arrows, Home/End,
/// Enter, Tab) are forwarded as raw key-downs. Ctrl+Tab is the focus-traversal
/// command ([`traverse_focus_keys`]), so a Tab is forwarded to the page only when
/// Ctrl is not held.
fn forward_keyboard_text(
    host: Option<NonSendMut<PaneHost>>,
    keycodes: Res<ButtonInput<KeyCode>>,
    mut keys: MessageReader<KeyboardInput>,
) {
    // Read-only in `host`: it forwards text into views (all `&self` calls) and
    // never changes the layout or focus, so it does not need `NonSendMut`'s
    // mutable deref.
    let Some(host) = host else {
        return;
    };
    let ctrl = keycodes.pressed(KeyCode::ControlLeft) || keycodes.pressed(KeyCode::ControlRight);
    let focused = host.focus.focused().and_then(|p| host.index_of(p));
    for key in keys.read() {
        if !key.state.is_pressed() {
            continue;
        }
        let Some(index) = focused else {
            continue;
        };
        let view = &host.windows[index].surface.view;
        match &key.logical_key {
            Key::Character(text) => view.key_char(text),
            Key::Space => view.key_char(" "),
            Key::Backspace => view.key(
                KeyEventType::RawKeyDown,
                VirtualKeyCode::Back,
                0,
                Modifiers::default(),
            ),
            Key::Enter => view.key(
                KeyEventType::RawKeyDown,
                VirtualKeyCode::Return,
                0,
                Modifiers::default(),
            ),
            // Caret movement and forward-delete: without these the caret cannot
            // move within a field and forward-delete is unavailable, so a comms
            // reply or a waypoint name can only be typed and back-spaced. Each
            // maps cleanly to an Ultralight virtual key and is forwarded as a raw
            // key-down like the arms above (issue #1124).
            Key::ArrowLeft => view.key(
                KeyEventType::RawKeyDown,
                VirtualKeyCode::Left,
                0,
                Modifiers::default(),
            ),
            Key::ArrowRight => view.key(
                KeyEventType::RawKeyDown,
                VirtualKeyCode::Right,
                0,
                Modifiers::default(),
            ),
            Key::ArrowUp => view.key(
                KeyEventType::RawKeyDown,
                VirtualKeyCode::Up,
                0,
                Modifiers::default(),
            ),
            Key::ArrowDown => view.key(
                KeyEventType::RawKeyDown,
                VirtualKeyCode::Down,
                0,
                Modifiers::default(),
            ),
            Key::Home => view.key(
                KeyEventType::RawKeyDown,
                VirtualKeyCode::Home,
                0,
                Modifiers::default(),
            ),
            Key::End => view.key(
                KeyEventType::RawKeyDown,
                VirtualKeyCode::End,
                0,
                Modifiers::default(),
            ),
            Key::Delete => view.key(
                KeyEventType::RawKeyDown,
                VirtualKeyCode::Delete,
                0,
                Modifiers::default(),
            ),
            // Ctrl+Tab is inter-pane focus; a bare Tab is the page's own field
            // traversal.
            Key::Tab if !ctrl => view.key(
                KeyEventType::RawKeyDown,
                VirtualKeyCode::Tab,
                0,
                Modifiers::default(),
            ),
            _ => {}
        }
    }
}

/// Keep the visible focus indicator on the focused pane (acceptance criterion 2).
///
/// Draws nothing when no pane is focused, and moves the reticle by despawning it
/// and respawning it over the newly-focused pane — which also re-homes it onto
/// the right Station window's camera when focus crosses windows, without mutating
/// a live component. The indicator is a ring plus four corner brackets: see the
/// `FOCUS_RING_*` constants for why that shape is what makes focus legible
/// without relying on colour.
fn update_focus_indicator(host: Option<NonSendMut<PaneHost>>, mut commands: Commands) {
    let Some(mut host) = host else {
        return;
    };
    let focused = host.focus.focused();
    let current = host.ring.map(|(pane, _)| pane);
    if focused == current {
        return;
    }
    if let Some((_, entity)) = host.ring.take() {
        commands.entity(entity).try_despawn();
    }
    if let Some(pane) = focused {
        if let Some(index) = host.index_of(pane) {
            let window = &host.windows[index];
            let entity = spawn_focus_ring(&mut commands, window);
            host.ring = Some((pane, entity));
        }
    }
}

/// Spawn the focus reticle over one pane: a full ring and four corner brackets,
/// on the pane's own window camera.
fn spawn_focus_ring(commands: &mut Commands, window: &PaneWindow) -> Entity {
    let scale = window.scale as f32;
    let left = window.origin.0 as f32 / scale + FOCUS_RING_INSET_PX;
    let top = window.origin.1 as f32 / scale + FOCUS_RING_INSET_PX;
    let width = (window.size.0 as f32 / scale - 2.0 * FOCUS_RING_INSET_PX).max(0.0);
    let height = (window.size.1 as f32 / scale - 2.0 * FOCUS_RING_INSET_PX).max(0.0);
    let color = focus_ring_color();

    let ring = commands
        .spawn((
            FocusRingNode,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(left),
                top: Val::Px(top),
                width: Val::Px(width),
                height: Val::Px(height),
                border: UiRect::all(Val::Px(FOCUS_RING_THICKNESS_PX)),
                ..default()
            },
            BorderColor::all(color),
            BackgroundColor(Color::NONE),
            // Above the pane's own texture.
            ZIndex(50),
        ))
        .id();

    // The four corner brackets, each an L of two thick borders, so the reticle
    // reads as a deliberate focus frame rather than a plain rectangle.
    for (h_side, v_side) in [
        (true, true),   // top-left
        (false, true),  // top-right
        (true, false),  // bottom-left
        (false, false), // bottom-right
    ] {
        let mut node = Node {
            position_type: PositionType::Absolute,
            width: Val::Px(FOCUS_RING_BRACKET_PX),
            height: Val::Px(FOCUS_RING_BRACKET_PX),
            ..default()
        };
        let mut border = UiRect::default();
        if v_side {
            node.top = Val::Px(0.0);
            border.top = Val::Px(FOCUS_RING_THICKNESS_PX);
        } else {
            node.bottom = Val::Px(0.0);
            border.bottom = Val::Px(FOCUS_RING_THICKNESS_PX);
        }
        if h_side {
            node.left = Val::Px(0.0);
            border.left = Val::Px(FOCUS_RING_THICKNESS_PX);
        } else {
            node.right = Val::Px(0.0);
            border.right = Val::Px(FOCUS_RING_THICKNESS_PX);
        }
        node.border = border;
        let bracket = commands
            .spawn((FocusRingNode, node, BorderColor::all(color), ZIndex(51)))
            .id();
        commands.entity(ring).add_child(bracket);
    }

    if let Some(cam) = window.station_camera {
        commands.entity(ring).insert(UiTargetCamera(cam));
    }
    ring
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
    // is pumped or drawn. A view left behind is not inert: the input systems
    // would still route to it, `pump_pane` would still drain a live page's
    // records into a registry that refuses them, once per frame, for the rest of
    // the run.
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
            None if outcome.recreation_exhausted => crate::pwarn!(
                log,
                LogCat::Lobby,
                "pane host: {} {} — and it has crashed too many times in too short a window, so \
                 it is left closed on AI control for the operator to repair rather than rebuilt \
                 into the same crash",
                outcome.failed,
                outcome.fault.reason()
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
    let mut recreated_any = false;
    for (new_id, url) in bus.0.take_pending_views() {
        // A display loss (issue #1125's other half) can close a just-recreated
        // pane in the frame between `recreate` queuing it here and this build.
        // `name_of`/the registry still resolve a closed pane's lingering record,
        // so build a view only for a pane that is still open — otherwise this
        // would leave an orphan surface nothing talks to. The pending entry is
        // already drained, so skipping drops it.
        if !bus.0.is_open(new_id) {
            crate::pinfo!(
                log,
                LogCat::Lobby,
                "pane host: recreated {new_id} was closed before its view was built (a display \
                 loss in the interval); not rebuilding it"
            );
            continue;
        }
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
            host.primary_window,
            slot.origin,
            slot.size,
            host.scale,
        ) {
            Ok(window) => {
                host.windows.push(window);
                recreated_any = true;
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
    // A recreated pane is a new surface on the same identity: rebuild the router
    // and reconcile the focus order so #1124's input routing reaches it.
    if recreated_any {
        host.rebuild_layout();
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
///
/// The recreated pane returns tiled on `window` (the primary viewscreen window)
/// on the game's default UI camera — the honest recovery seat; it does not
/// re-composite onto a Station window, whose 2-D camera the close path may have
/// already despawned.
fn make_pane_view(
    runtime: &UltralightRuntime,
    images: &mut Assets<Image>,
    commands: &mut Commands,
    id: PaneId,
    url: &str,
    window: Entity,
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
        window,
        station_camera: None,
        origin,
        size,
        scale,
        window_origin: (0, 0),
        copy_failures: 0,
    })
}

/// Tear down the view of every pane the bus no longer lists as open, and
/// reconcile the routing models with what remains.
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
    let mut closed: Vec<PaneId> = Vec::new();
    host.windows.retain(|window| {
        if open.contains(&window.id) {
            return true;
        }
        commands.entity(window.canvas).try_despawn();
        closed.push(window.id);
        crate::pinfo!(
            log,
            LogCat::Lobby,
            "pane host: {} is closed — its view is torn down and its station is \
             the lobby's business now",
            window.id
        );
        false
    });

    // A finger pinned to a pane that has gone must not stay captured by a view
    // that no longer exists; and the focus indicator over a closed pane must go
    // with it.
    for pane in &closed {
        host.contacts.release_pane(*pane);
        if host.ring.map(|(p, _)| p) == Some(*pane) {
            if let Some((_, entity)) = host.ring.take() {
                commands.entity(entity).try_despawn();
            }
        }
    }
    // A Station window whose every pane has closed no longer needs its 2-D
    // camera; despawn it so nothing keeps clearing an empty Station to black. The
    // window itself is `bridge_display`'s to own.
    host.station_cameras.retain(|(window, camera)| {
        if host.windows.iter().any(|w| w.window == *window) {
            return true;
        }
        commands.entity(*camera).try_despawn();
        false
    });

    // Rebuild the router and reconcile the focus order. `sync_order` keeps the
    // focused pane if it survived and clears focus if it closed — never carrying
    // one participant's focus onto another. If focus was cleared, the indicator
    // is dropped on the next `update_focus_indicator` frame.
    let previously_focused = host.focus.focused();
    host.rebuild_layout();
    if host.focus.focused().is_none() && previously_focused.is_some() {
        if let Some((_, entity)) = host.ring.take() {
            commands.entity(entity).try_despawn();
        }
    }
}
