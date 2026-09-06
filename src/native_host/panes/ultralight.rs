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

use std::time::Instant;

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

use super::document::pane_drain_script;
use super::frame_stats::{PaneExperiments, PaneFrameSample, PaneFrameStats};
use super::placement::{home_for_pane, PaneHome, PaneTile};
use super::recovery::{service_faults, PaneFault};
use super::registry::PaneId;
use super::surface::{pump_pane, PaneSurface, PaneSurfaceError};
use super::PaneBusResource;
use crate::console_bridge::HudStateChanged;
use crate::core::messages::GamePhase;
use crate::logging::{LogCat, LogFilterConfig};
use crate::native_host::bridge_display::{BridgeLayoutResource, BridgeStationSurfaces};
use crate::native_host::bridge_layout::BridgeLayout;
use crate::native_host::bridge_profile::PaneRect;
use crate::native_host::host_lobby::{
    host_lobby_drain_script, pump_host_lobby, HostLobbyBridgeResource, HostLobbyRevealResource,
    HOST_LOBBY_SURFACE_ID,
};
use crate::native_host::input_routing::{
    pointer_follow_focus, ContactCaptureMap, FocusRing, MouseCapture, PaneHit, PanePlacement,
    PaneRouter, PointerMotion, WindowKey,
};

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

/// The native host's own lobby surface (issue #1325), and where it loads from.
///
/// Present when `phoenix-host` published a lobby document — see
/// [`LocalHostLobby`](crate::native_host::host_lobby::LocalHostLobby). Absent
/// for a host with no bundle to build one from, which then runs exactly as it
/// did before.
///
/// A resource of its own rather than another [`PaneDisplayEntry`], because the
/// surface is not a pane: it has no participant label to seat it by, no identity
/// in its URL, and it always takes the whole primary window rather than a slot.
/// What it *does* share is everything below the URL — the one Ultralight
/// runtime, the texture, the compositing node and the input router — which is
/// why it is driven by [`PaneHost`] and not by a second host of its own.
#[derive(Resource, Clone, Debug)]
pub struct HostLobbyDisplayConfig {
    pub url: String,
}

/// The viewscreen HUD-overlay surface's display config (issue #422's
/// `#hud-overlay`, ported to the native path). Like [`HostLobbyDisplayConfig`] it
/// carries only the URL the surface's view loads; the surface itself is
/// composited and driven by [`PaneHost`] — a TRANSPARENT surface on the
/// viewscreen window, drawn over the 3-D scene, shown only in-game.
#[derive(Resource, Clone, Debug)]
pub struct ViewscreenHudDisplayConfig {
    pub url: String,
}

/// The latest HUD state pushed to the viewscreen overlay, cached so a state that
/// arrives before the surface has finished loading still reaches it, and so the
/// overlay always reflects the newest state each frame it is drawn.
#[derive(Resource, Clone, Debug, Default)]
struct ViewscreenHudLatest {
    json: Option<String>,
}

/// The viewscreen HUD-overlay surface's reserved id. Like
/// [`HOST_LOBBY_SURFACE_ID`](crate::native_host::host_lobby::HOST_LOBBY_SURFACE_ID)
/// (`u32::MAX`) it sits at the top of the `PaneId` space the registry — which
/// mints from `0` upward and never reuses — cannot reach, so it can never collide
/// with a participant pane.
pub const VIEWSCREEN_HUD_SURFACE_ID: PaneId = PaneId(u32::MAX - 1);

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

/// One embedded view, behind the trait the frame loop drives.
pub struct UltralightPaneSurface {
    view: UltralightPane,
    /// Set once the document has reported a completed load. Sticky: `is_loading`
    /// goes false between navigations too, and a pane navigates once.
    loaded: bool,
    /// The script that collects what this document has queued.
    ///
    /// Held per surface because the two documents this type drives install
    /// **different** queues: a pane's is `phoenixPaneOut` (a participant's
    /// `ClientMessage`s, admitted as such) and the host lobby's is
    /// `phoenixHostLobbyOut` (the host operator's own picks and this machine's
    /// own screen arrangement, judged by the arbiter and the layout law and
    /// never admitted at all). Two namespaces make a record that arrived on the
    /// wrong one unrepresentable rather than merely wrong — see
    /// `host_lobby::document::HOST_LOBBY_OUT_NAMESPACE` — and a surface that
    /// drained the wrong one would simply find no function and report nothing,
    /// forever, with a clean log. That cost nothing until issue #1328 gave the
    /// lobby surface something to say and #1330 gave it more; this field is what
    /// makes the mistake unrepresentable.
    drain_script: String,
}

impl UltralightPaneSurface {
    /// Wrap a freshly created **pane** view.
    ///
    /// Public so an integration test can drive one pane without a Bevy `App`:
    /// the automated proof that a real console page loads and answers
    /// (`tests/native_host_pane_ultralight.rs`) needs a runtime, a view and this
    /// wrapper, and nothing else this module builds.
    pub fn new(view: UltralightPane) -> Self {
        Self {
            view,
            loaded: false,
            drain_script: pane_drain_script(),
        }
    }

    /// Wrap a freshly created **host-lobby** view (issues #1325/#1328/#1330).
    ///
    /// Everything below the queue is identical to a pane's — the same runtime,
    /// the same texture, the same push primitive — so this is a constructor
    /// rather than a second type. What differs is the one thing that must:
    /// which page→host queue it drains. See [`Self::drain_script`].
    pub fn for_host_lobby(view: UltralightPane) -> Self {
        Self {
            view,
            loaded: false,
            drain_script: host_lobby_drain_script(),
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
        match self.view.evaluate(&self.drain_script) {
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

impl PaneWindow {
    /// Whether this window is the host-lobby surface rather than a participant's
    /// pane (issue #1325).
    ///
    /// The surface shares the runtime, the texture, the compositing node and the
    /// input router with the panes, and shares nothing above that: it has no
    /// entry in the pane bus, so everything that treats a `PaneId` as a
    /// participant — the pump, the fault path, the close-and-retire sweep —
    /// asks this first. See
    /// [`HOST_LOBBY_SURFACE_ID`](crate::native_host::host_lobby::HOST_LOBBY_SURFACE_ID)
    /// for why the handle is reserved rather than minted.
    fn is_host_lobby(&self) -> bool {
        self.id == HOST_LOBBY_SURFACE_ID
    }

    /// Whether this is the viewscreen HUD-overlay surface (issue #422, native
    /// port) rather than a participant's pane or the lobby. Like the lobby it has
    /// no pane-bus entry; unlike the lobby it is a passive `pointer-events:none`
    /// overlay, so it is also kept out of the input router. See
    /// [`VIEWSCREEN_HUD_SURFACE_ID`].
    fn is_hud_overlay(&self) -> bool {
        self.id == VIEWSCREEN_HUD_SURFACE_ID
    }
}

/// The Ultralight runtime and every pane hanging off it.
///
/// `!Send` by construction — `Renderer` and `View` are raw pointers with thread
/// affinity — so it is a non-send resource and only main-thread systems touch
/// it.
pub struct PaneHost {
    runtime: UltralightRuntime,
    windows: Vec<PaneWindow>,
    /// Each **tiled** pane's slot on the primary window, so a recreated pane
    /// rebuilds where its predecessor sat (issue #1125).
    ///
    /// Built once at init, and only for a pane genuinely tiled on the primary
    /// window: a pane the profile seated on a Station window has its home in
    /// [`BridgeStationSurfaces`] instead, and storing that monitor's rectangle
    /// here would describe a viewscreen tile nobody ever wanted (issue #1333).
    /// [`home_for_pane`] is what reads this, and it is the last of the four
    /// homes it considers.
    tiles: Vec<PaneTile>,
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
    /// One 2-D camera per Station window, `(window, camera)` — spawned once and
    /// reused as panes are composited onto that window.
    station_cameras: Vec<(Entity, Entity)>,
    /// Whether the host-lobby surface (issue #1325) is currently placed in the
    /// router and drawn.
    ///
    /// Tracked so the router is rebuilt on the *edge* rather than every frame:
    /// `rebuild_layout` re-derives the focus order, and doing that sixty times a
    /// second would fight a Ctrl+Tab the operator just made.
    lobby_present: bool,
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
        self.router = build_router(&self.windows, self.lobby_present);
        self.focus.sync_order(self.router.focus_order());
    }
}

/// Build the pure router from the live pane windows.
///
/// The host-lobby surface (issue #1325) is placed **only while it is on
/// screen**: with no placement, a pointer or a touch over that region resolves
/// to no pane at all and is left to the viewscreen, which is the whole of
/// "input-transparent when the chrome has yielded". It is also placed **last**,
/// so that where a tiled pane overlaps it — panes draw on top — the pane wins
/// the hit test, the router resolving to the first placement that contains the
/// point.
fn build_router(windows: &[PaneWindow], lobby_present: bool) -> PaneRouter {
    PaneRouter::new(
        windows
            .iter()
            // The HUD overlay (issue #422, native port) is never routed — it is a
            // passive `pointer-events:none` frame over the live viewscreen, so a
            // pointer or touch over it must reach the game beneath, exactly as an
            // un-placed lobby surface does.
            .filter(|w| (lobby_present || !w.is_host_lobby()) && !w.is_hud_overlay())
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
        app.init_resource::<ViewscreenHudLatest>();
        app.add_systems(PreUpdate, init_pane_host).add_systems(
            Update,
            (
                // Before the input group: whether the lobby surface is placed in
                // the router at all is decided here, and a click this frame must
                // be routed against this frame's answer.
                sync_host_lobby_presence,
                // Also before input and before the frame copy: a resize moves the
                // views and re-tiles, and both the router and `drive_panes` must
                // see this frame's rects. Gated like `drive_panes` — no image
                // assets means no surfaces to resize (the Contract host).
                resize_pane_surfaces.run_if(resource_exists::<Assets<Image>>),
                (
                    route_pointer_input,
                    route_touch_input,
                    traverse_focus_keys,
                    forward_keyboard_text,
                )
                    .chain()
                    .run_if(resource_exists::<ButtonInput<MouseButton>>),
                // Reveal the HUD overlay only in-game, and cache the newest HUD
                // state (issue #422, native port) — both BEFORE `drive_panes`, so
                // the frame it draws this tick shows the right presence and the
                // newest readout. `cache_hud_state` is gated on the host actually
                // registering `HudStateChanged`, so the rendererless Contract test
                // (which has no server plugins) stands the pane host up without it.
                sync_viewscreen_hud_presence,
                cache_hud_state
                    .run_if(resource_exists::<bevy::ecs::message::Messages<HudStateChanged>>),
                // Feed the host's gamepads into each console pane (native gamepad
                // route) BEFORE `drive_panes`, so the snapshot the page polls this
                // tick is current. Gated on the input plugin so the rendererless
                // Contract host — which has neither input nor gilrs — skips it.
                push_gamepads_to_panes.run_if(resource_exists::<ButtonInput<MouseButton>>),
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
        // Either kind of surface is reason enough to stand the host up: a host
        // with no `--pane` still shows the lobby (issue #1325), and a host with
        // panes and no bundle to build a lobby from still shows the panes.
        || (world.get_resource::<PaneDisplayConfig>().is_none()
            && world.get_resource::<HostLobbyDisplayConfig>().is_none())
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

    let config = world
        .get_resource::<PaneDisplayConfig>()
        .cloned()
        .unwrap_or(PaneDisplayConfig { panes: Vec::new() });
    let lobby_config = world.get_resource::<HostLobbyDisplayConfig>().cloned();
    let hud_config = world.get_resource::<ViewscreenHudDisplayConfig>().cloned();
    if config.panes.is_empty() && lobby_config.is_none() {
        world.insert_resource(PaneHostFailed);
        return;
    }
    // The bus itself is read per frame by `drive_panes`; what matters here is
    // that there IS one, because a pane host with no bus would draw consoles
    // nothing could talk to. Only PANES need it — the lobby surface is not a
    // participant and speaks over its own bridge (issue #1325) — so a
    // lobby-only host is not held to it.
    if !config.panes.is_empty() && world.get_resource::<PaneBusResource>().is_none() {
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
        /// The host-lobby surface rather than a participant's pane (#1325).
        lobby: bool,
        /// The viewscreen HUD-overlay surface (issue #422, native port) — a
        /// transparent frame over the 3-D viewscreen, shown in-game only and
        /// never routed input.
        hud: bool,
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
                    lobby: false,
                    hud: false,
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
                lobby: false,
                hud: false,
            });
        }
    }
    // The host-lobby surface takes the whole primary window, and is seated
    // LAST (issue #1325). Last is what puts it last in the input router, so
    // that where a tiled pane overlaps it the pane wins the hit test — the
    // router resolves to the first placement containing the point, and a pane
    // is what the operator is looking at there. Its DRAW order is the other way
    // round and is set by an explicit `ZIndex` below, not by this order.
    if let Some(lobby) = &lobby_config {
        seats.push(Seat {
            entry: PaneDisplayEntry {
                id: HOST_LOBBY_SURFACE_ID,
                url: lobby.url.clone(),
                label: String::new(),
            },
            window: primary_entity,
            origin: (0, 0),
            size: (primary_width, primary_height),
            scale: primary_scale,
            window_origin: (0, 0),
            station: false,
            lobby: true,
            hud: false,
        });
    }
    // The viewscreen HUD overlay (issue #422, native port) also takes the whole
    // primary window, TRANSPARENT, so it frames the 3-D viewscreen the way
    // `server.html` frames its canvas. It never enters the input router (a
    // passive `pointer-events:none` overlay) and is shown only in-game
    // (`sync_viewscreen_hud_presence`), so it never fights the lobby for the
    // window: one is drawn while the other is hidden by phase.
    if let Some(hud) = &hud_config {
        seats.push(Seat {
            entry: PaneDisplayEntry {
                id: VIEWSCREEN_HUD_SURFACE_ID,
                url: hud.url.clone(),
                label: String::new(),
            },
            window: primary_entity,
            origin: (0, 0),
            size: (primary_width, primary_height),
            scale: primary_scale,
            window_origin: (0, 0),
            station: false,
            lobby: false,
            hud: true,
        });
    }

    // Whether the lobby chrome is on screen right now. `Lobby` is
    // `GamePhase::default()`, so a host that has not inserted the resource yet
    // is showing it — which is what a host boots into.
    let lobby_present = world
        .get_resource::<HostLobbyRevealResource>()
        .map(|r| r.0.presence().composited)
        .unwrap_or(true);

    let runtime = match UltralightRuntime::start(&RuntimeOptions::default()) {
        Ok(runtime) => runtime,
        Err(e) => {
            crate::perror!(log, LogCat::Lobby, "pane host: {e}");
            world.insert_resource(PaneHostFailed);
            return;
        }
    };

    // The bus, for resolving each pane's participant name into the layout that a
    // recreated pane (issue #1125) rebuilds against. Absent on a lobby-only
    // host, which has no participants to resolve.
    let bus = world.get_resource::<PaneBusResource>().cloned();

    let mut windows = Vec::new();
    let mut station_cameras: Vec<(Entity, Entity)> = Vec::new();
    let mut tiles: Vec<PaneTile> = Vec::new();
    for seat in seats {
        let id = seat.entry.id;
        let url = seat.entry.url.as_str();
        // Record this pane's tile by its stable participant name, so a pane
        // recreated after a view crash reopens in the same place (issue #1125).
        //
        // ONLY for a pane genuinely tiled on the primary window (issue #1333). A
        // pane the profile seated on a Station window has its home in the live
        // `BridgeStationSurfaces`, which follows it when the layout moves; the
        // rectangle it occupies THERE is measured on that monitor, and recording
        // it here would describe a strip of the viewscreen the profile never
        // asked for. `home_for_pane` refuses to tile a seated console anyway —
        // this is the trap removed rather than merely guarded.
        if !seat.station {
            if let Some(name) = bus.as_ref().and_then(|b| b.0.name_of(id)) {
                tiles.push(PaneTile {
                    name,
                    origin: seat.origin,
                    size: seat.size,
                });
            }
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
            // The HUD overlay (issue #422, native port) renders with alpha so the
            // 3-D viewscreen shows through everywhere its frame does not paint;
            // every other surface is opaque console chrome.
            transparent: seat.hud,
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
        // The lobby surface drains its OWN queue — the one place the two
        // surfaces differ below the URL. Sharing this too would leave its picks
        // and its monitor buttons evaluating `window.__phoenixPaneOutDrain`, a
        // function its document never installs, so every press would be
        // swallowed with a clean log.
        let mut surface = if seat.lobby {
            UltralightPaneSurface::for_host_lobby(view)
        } else {
            UltralightPaneSurface::new(view)
        };
        if let Err(e) = surface.load(url) {
            crate::perror!(
                log,
                LogCat::Lobby,
                "pane host: {id} could not load its console: {e}"
            );
            world.insert_resource(PaneHostFailed);
            return;
        }
        // The transparent HUD overlay starts fully clear so the one frame before
        // its first copy shows the 3-D scene, not a black fill; opaque surfaces
        // start black.
        let fill: [u8; 4] = if seat.hud {
            [0, 0, 0, 0]
        } else {
            [0, 0, 0, 255]
        };
        let image = Image::new_fill(
            Extent3d {
                width: seat.size.0,
                height: seat.size.1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &fill,
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
                    // The lobby surface's INVISIBLE half (issue #1325). The
                    // view stays alive and keeps being pushed to; it simply is
                    // not drawn, which is what makes "the chrome yielded"
                    // different from "the surface was torn down". The HUD overlay
                    // (issue #422, native port) starts hidden too — the host boots
                    // into the lobby, and `sync_viewscreen_hud_presence` reveals
                    // the frame once the mission is InProgress.
                    display: if (seat.lobby && !lobby_present) || seat.hud {
                        Display::None
                    } else {
                        Display::DEFAULT
                    },
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
        // The lobby surface draws BENEATH every pane on the same window. Its
        // seat is last (which is what puts it last in the input router), so the
        // draw order has to be said explicitly rather than inherited from spawn
        // order. Negative only orders it within the UI pass — the UI still
        // draws over the 3-D viewscreen, which is the point of the surface.
        if seat.lobby {
            world.entity_mut(canvas).insert(ZIndex(-1));
        }
        // The HUD overlay (issue #422, native port) draws OVER the 3-D viewscreen
        // and above any other UI on the window — the lobby sits at -1, and a
        // running mission has no panes tiled on the viewscreen. A positive index
        // keeps the frame on top wherever it is shown.
        if seat.hud {
            world.entity_mut(canvas).insert(ZIndex(20));
        }
        windows.push(PaneWindow {
            id,
            surface,
            image: handle,
            canvas,
            window: seat.window,
            origin: seat.origin,
            size: seat.size,
            scale: seat.scale,
            window_origin: seat.window_origin,
            copy_failures: 0,
        });
        if seat.lobby {
            crate::pinfo!(
                log,
                LogCat::Lobby,
                "pane host: the host lobby is on the viewscreen window at {}x{} ({})",
                seat.size.0,
                seat.size.1,
                if lobby_present { "showing" } else { "yielded" },
            );
        } else {
            // The URL is NOT logged: it carries this pane's session token in
            // its fragment, and an operator log is a file, a scrollback and a
            // screenshot. `phoenix-host` prints the first eight characters of
            // the token when it opens the pane, which is enough to correlate.
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
    }

    let router = build_router(&windows, lobby_present);
    // Seed focus onto the first pane so a pure-keyboard operator sees the reticle
    // and has a defined keyboard target the instant panes exist, rather than a
    // blank ring in which keys go nowhere until the first Ctrl+Tab (issue #1124,
    // acceptance criterion 2). The Ultralight view is told to focus to match, so
    // the first keystroke lands without a preceding click or Ctrl+Tab —
    // Ultralight drops input into an unfocused view.
    //
    // The first *pane*, not the first entry: `focused_on_first_pane` skips the
    // host-lobby surface, which carries no typeable control and so would be
    // framed by a reticle promising a keyboard target that accepts nothing. A
    // host with no `--pane` seeds no focus at all, which is the honest state.
    let focus = FocusRing::focused_on_first_pane(router.focus_order());
    if let Some(first) = focus.focused() {
        if let Some(window) = windows.iter().find(|w| w.id == first) {
            window.surface.view.focus();
        }
    }
    world.insert_non_send_resource(PaneHost {
        runtime,
        windows,
        tiles,
        primary_window: primary_entity,
        scale: primary_scale,
        focus,
        contacts: ContactCaptureMap::new(),
        mouse_capture: MouseCapture::new(),
        pointer_motion: PointerMotion::new(),
        router,
        station_cameras,
        lobby_present,
    });
}

/// Show or hide the host-lobby surface, and place or unplace it in the input
/// router (issue #1325).
///
/// The three observable halves of one decision, taken by the pure
/// [`RevealState`](crate::native_host::host_lobby::RevealState): the node's
/// `display` is the **invisible** half, the router placement is the
/// **input-transparent** half, and the third — telling the page to render chrome
/// its own phase would hide — is pushed over the bridge by
/// `host_lobby::publish_reveal`.
///
/// The view itself is never touched. It stays alive, stays loaded and stays
/// pushed to, so a reveal is a `display` flip rather than a page load: that is
/// what "the surface is permanent" buys, and what later slices (a QR overlay,
/// settings, layout rows) are entitled to assume.
///
/// Runs only on the EDGE. `rebuild_layout` re-derives the keyboard focus order,
/// and re-deriving it every frame would revert a Ctrl+Tab the operator just
/// made — the same reason focus follows the pointer only on genuine motion.
fn sync_host_lobby_presence(
    host: Option<NonSendMut<PaneHost>>,
    reveal: Option<Res<HostLobbyRevealResource>>,
    mut nodes: Query<&mut Node>,
) {
    let (Some(mut host), Some(reveal)) = (host, reveal) else {
        return;
    };
    let present = reveal.0.presence().composited;
    if host.lobby_present == present {
        return;
    }
    let Some(canvas) = host
        .windows
        .iter()
        .find(|w| w.is_host_lobby())
        .map(|w| w.canvas)
    else {
        // No lobby surface on this host (no bundle to build one from). Record
        // the answer anyway so this does not re-run every frame.
        host.lobby_present = present;
        return;
    };
    host.lobby_present = present;
    if let Ok(mut node) = nodes.get_mut(canvas) {
        node.display = if present {
            Display::DEFAULT
        } else {
            Display::None
        };
    }

    // Re-place (or un-place) it in the router, and reconcile focus. `sync_order`
    // clears focus when the focused surface leaves the order rather than
    // carrying it onto whatever now occupies that position.
    //
    // A reveal does NOT seed focus onto the surface. Same rule as
    // `FocusRing::focused_on_first_pane`: keyboard focus is a promise that the
    // next keystroke lands somewhere, and this surface has no control to land it
    // in, so seeding it would route input at chrome that accepts nothing. It
    // stays in the focus order, so a deliberate Ctrl+Tab still reaches it.
    let previously_focused = host.focus.focused();
    host.rebuild_layout();
    // Ultralight drops input into an unfocused view, so the views follow the
    // model rather than the other way round.
    let next = host.focus.focused();
    host.focus_view(previously_focused, next);
}

/// Cache the newest viewscreen HUD state (issue #422's `#hud-overlay`, ported to
/// the native path) from the host's `HudStateChanged`.
///
/// The host emits `HudStateChanged` only on a real change (heading, hull,
/// condition, red alert). Caching the latest — rather than pushing it straight
/// through — lets [`drive_panes`] hand it to the overlay every frame it draws,
/// so a state that arrived while the transparent surface was still loading, and
/// the very first frame after it finishes loading, both reach it.
fn cache_hud_state(
    mut latest: ResMut<ViewscreenHudLatest>,
    mut events: MessageReader<HudStateChanged>,
) {
    for event in events.read() {
        latest.json = Some(event.json.clone());
    }
}

/// Show the viewscreen HUD overlay (issue #422, native port) only while a mission
/// is `InProgress`, and hide it otherwise.
///
/// The host boots into `Lobby` with the crew-lobby surface on the viewscreen; the
/// HUD frame belongs over the LIVE 3-D scene, so it and the lobby take the same
/// window but are never drawn at once — one is hidden by phase while the other
/// shows, the same mutual exclusion `server.html` gets for free by swapping which
/// element it displays.
fn sync_viewscreen_hud_presence(
    host: Option<NonSend<PaneHost>>,
    phase: Option<Res<State<GamePhase>>>,
    mut nodes: Query<&mut Node>,
) {
    let (Some(host), Some(phase)) = (host, phase) else {
        return;
    };
    let Some(canvas) = host
        .windows
        .iter()
        .find(|w| w.is_hud_overlay())
        .map(|w| w.canvas)
    else {
        return;
    };
    // Shown in play AND at game over: the frame stays up while the mission runs,
    // and the same surface carries the game-over screen when it ends (its
    // `#game-over-overlay`, revealed by `__updateHud` from `game_over_message`).
    // Hiding it in `GameOver` would blank the ending, so both phases keep it.
    let want = if matches!(*phase.get(), GamePhase::InProgress | GamePhase::GameOver) {
        Display::DEFAULT
    } else {
        Display::None
    };
    if let Ok(mut node) = nodes.get_mut(canvas) {
        // Write only on a real change so an unchanged phase does not dirty the UI
        // layout every frame.
        if node.display != want {
            node.display = want;
        }
    }
}

/// Stable browser-slot numbering for the host's gamepads (native gamepad route).
///
/// The page's per-station selection setting stores a slot index, so a slot must
/// stay pointed at the same physical pad for the session: a `gilrs` entity keeps
/// the slot it was first seen on. `had_any` lets the feeder push one clearing
/// snapshot when the last pad leaves and then fall quiet.
#[derive(Default)]
struct GamepadSlots {
    map: std::collections::HashMap<Entity, usize>,
    next: usize,
    had_any: bool,
}

/// Read every connected gilrs gamepad into the W3C "standard" `PadReading` shape
/// — buttons in W3C order, and the stick Y axes negated to the W3C convention
/// (positive is DOWN, where Bevy's stick Y is positive UP).
fn read_pads(
    pads: &Query<(Entity, &Gamepad)>,
    slots: &mut GamepadSlots,
) -> Vec<super::gamepad::PadReading> {
    const W3C_BUTTONS: [GamepadButton; super::gamepad::STANDARD_BUTTONS] = [
        GamepadButton::South,
        GamepadButton::East,
        GamepadButton::West,
        GamepadButton::North,
        GamepadButton::LeftTrigger,
        GamepadButton::RightTrigger,
        GamepadButton::LeftTrigger2,
        GamepadButton::RightTrigger2,
        GamepadButton::Select,
        GamepadButton::Start,
        GamepadButton::LeftThumb,
        GamepadButton::RightThumb,
        GamepadButton::DPadUp,
        GamepadButton::DPadDown,
        GamepadButton::DPadLeft,
        GamepadButton::DPadRight,
    ];
    let mut out = Vec::new();
    for (entity, pad) in pads.iter() {
        let slot = match slots.map.get(&entity) {
            Some(slot) => *slot,
            None => {
                let slot = slots.next;
                slots.next += 1;
                slots.map.insert(entity, slot);
                slot
            }
        };
        let mut buttons = [(false, 0.0f32); super::gamepad::STANDARD_BUTTONS];
        for (i, button) in W3C_BUTTONS.into_iter().enumerate() {
            buttons[i] = (pad.pressed(button), pad.get(button).unwrap_or(0.0));
        }
        let axes = [
            pad.get(GamepadAxis::LeftStickX).unwrap_or(0.0),
            -pad.get(GamepadAxis::LeftStickY).unwrap_or(0.0),
            pad.get(GamepadAxis::RightStickX).unwrap_or(0.0),
            -pad.get(GamepadAxis::RightStickY).unwrap_or(0.0),
        ];
        out.push(super::gamepad::PadReading {
            slot,
            buttons,
            axes,
        });
    }
    out
}

/// Feed every connected gamepad's live state into each console pane each frame
/// (native gamepad route). Ultralight has no Gamepad API, so `pane_boot.js`
/// installs a `navigator.getGamepads()` shim; this pushes the W3C-standard
/// snapshot it returns, and the client page's ordinary `gui/gamepad-input.js`
/// runtime does the per-pad selection and drives THIS pane's one station.
///
/// Only console panes get it — the host-lobby and HUD surfaces are not client
/// consoles and install no receiver. Pushed before `drive_panes` renders, so the
/// snapshot the page's `requestAnimationFrame` poll reads this tick is current.
fn push_gamepads_to_panes(
    host: Option<NonSendMut<PaneHost>>,
    pads: Query<(Entity, &Gamepad)>,
    mut slots: Local<GamepadSlots>,
) {
    let Some(mut host) = host else {
        return;
    };
    let readings = read_pads(&pads, &mut slots);
    // The steady no-pad state pushes nothing; the frame the last pad leaves
    // pushes one empty snapshot so the page sees it disconnect, then falls quiet.
    if readings.is_empty() && !slots.had_any {
        return;
    }
    slots.had_any = !readings.is_empty();
    let json = super::gamepad::gamepad_snapshot_json(&readings);
    let script = format!("window.__phoenixSetGamepads({json})");
    for pane in host.windows.iter_mut() {
        if pane.is_host_lobby() || pane.is_hud_overlay() {
            continue;
        }
        let _ = pane.surface.push(&script);
    }
}

/// Follow each OS window's size: resize every surface's Ultralight view and its
/// Bevy texture so the page reflows to the window, the way a browser viewport
/// does.
///
/// [`init_pane_host`] sizes each view once, from the window it finds at startup,
/// and never revisits it — so without this the lobby chrome and the consoles
/// stay frozen at their opening size when the operator resizes the window, and
/// Bevy's own window resize would leave the view (old size) and the window's
/// render target (new size) disagreeing. Runs every frame and does nothing until
/// a window's physical size actually changes.
///
/// The per-window layout mirrors [`init_pane_host`]'s: a lone surface fills its
/// window; several panes on one window tile side by side in their existing
/// order. The device scale is taken as unchanged (a resize within one monitor);
/// a drag onto a monitor of a different DPI would need the view rebuilt at the
/// new `device_scale`, which is out of scope here.
fn resize_pane_surfaces(
    host: Option<NonSendMut<PaneHost>>,
    windows: Query<&Window>,
    mut images: ResMut<Assets<Image>>,
    mut nodes: Query<&mut Node>,
) {
    let Some(mut host) = host else {
        return;
    };
    // Group pane indices by the window they sit on, preserving order — a pane's
    // tile index within its window is its position here, exactly as at init.
    let mut groups: Vec<(Entity, Vec<usize>)> = Vec::new();
    for (i, pane) in host.windows.iter().enumerate() {
        match groups.iter_mut().find(|(w, _)| *w == pane.window) {
            Some((_, idxs)) => idxs.push(i),
            None => groups.push((pane.window, vec![i])),
        }
    }
    // Target (origin, size) per pane in physical pixels within its window.
    let mut targets: Vec<Option<((u32, u32), (u32, u32))>> = vec![None; host.windows.len()];
    for (window_entity, idxs) in &groups {
        let Ok(window) = windows.get(*window_entity) else {
            continue;
        };
        let pw = window.physical_width().max(1);
        let ph = window.physical_height().max(1);
        // The host-lobby surface (issue #1325) and the HUD overlay (issue #422,
        // native port) are FULL-WINDOW overlays, not tiled consoles: each fills
        // the whole window and they stack by ZIndex (the lobby beneath, the HUD
        // above, shown one at a time by phase). Only genuine console panes tile
        // among themselves — counting the overlays in the tile split is what
        // squeezed the HUD into half the viewscreen.
        let tiled: Vec<usize> = idxs
            .iter()
            .copied()
            .filter(|&i| !host.windows[i].is_host_lobby() && !host.windows[i].is_hud_overlay())
            .collect();
        for &i in idxs {
            if host.windows[i].is_host_lobby() || host.windows[i].is_hud_overlay() {
                targets[i] = Some(((0, 0), (pw, ph)));
            }
        }
        if tiled.len() == 1 {
            targets[tiled[0]] = Some(((0, 0), (pw, ph)));
        } else if tiled.len() > 1 {
            let count = tiled.len() as u32;
            let tile_w = (pw / count).max(1);
            for (slot, &i) in tiled.iter().enumerate() {
                targets[i] = Some(((tile_w * slot as u32, 0), (tile_w, ph)));
            }
        }
    }
    let mut changed = false;
    for (i, pane) in host.windows.iter_mut().enumerate() {
        let Some((origin, size)) = targets[i] else {
            continue;
        };
        if origin == pane.origin && size == pane.size {
            continue;
        }
        // Move the view, reallocate the texture it copies into, and resize the
        // node that draws it — all three must agree, or `copy_frame` writes a
        // buffer of the wrong length into the image the next frame.
        pane.surface.view_mut().resize(size.0, size.1);
        if let Some(image) = images.get_mut(&pane.image) {
            image.resize(Extent3d {
                width: size.0,
                height: size.1,
                depth_or_array_layers: 1,
            });
        }
        if let Ok(mut node) = nodes.get_mut(pane.canvas) {
            node.left = Val::Px(origin.0 as f32 / pane.scale as f32);
            node.top = Val::Px(origin.1 as f32 / pane.scale as f32);
            node.width = Val::Px(size.0 as f32 / pane.scale as f32);
            node.height = Val::Px(size.1 as f32 / pane.scale as f32);
        }
        pane.origin = origin;
        pane.size = size;
        changed = true;
    }
    if changed {
        host.rebuild_layout();
    }
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
    // selection every frame, and never while the left button is captured: a
    // cross-pane drag must operate the pane it began on, so focus stays there
    // rather than chasing the pane the cursor happens to end over (a pointer
    // grab, as conventional focus-follows-mouse does).
    if host.mouse_capture.captured().is_none() {
        if let Some((key, x, y, Some(hit))) = cursor {
            let previous = host.focus.focused();
            let host = &mut *host;
            if pointer_follow_focus(
                &mut host.focus,
                &mut host.pointer_motion,
                key,
                x,
                y,
                hit.pane,
            ) {
                host.focus_view(previous, Some(hit.pane));
            }
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
                .filter(|(key, ..)| host.router.placement(captured).map(|p| p.window) == Some(*key))
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
/// model cycles; typed input then routes to whichever pane focus lands on.
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

/// One frame for every pane: service the library, move messages both ways,
/// rasterise, and copy what repainted into each pane's texture.
///
/// Runs on the Bevy main thread, which is the simulation's — see the module
/// note, and [`pump_pane`]'s per-frame push budget.
fn drive_panes(
    host: Option<NonSendMut<PaneHost>>,
    bus: Option<Res<PaneBusResource>>,
    lobby: Option<Res<HostLobbyBridgeResource>>,
    // The live Station surfaces (issue #1331): where a console the lobby just
    // opened is composited. Read rather than written — `follow_layout_stations`
    // owns them — and optional, because a `NativeRenderSurface::Contract` host
    // has no display adapter at all.
    stations: Option<Res<BridgeStationSurfaces>>,
    // The live bridge LAW (issue #1333), read for one question: does it seat a
    // console under this pane's name? A console with a screen of its own is
    // never rebuilt over the viewscreen, even in the frames where that screen
    // has no slot to offer — see `super::placement`.
    bridge: Option<Res<BridgeLayoutResource>>,
    // The latest viewscreen HUD state (issue #422, native port), cached from
    // `HudStateChanged` by `cache_hud_state`. Read here so the newest reaches the
    // overlay every frame it draws, including the first frame after it loads.
    hud_latest: Res<ViewscreenHudLatest>,
    mut images: ResMut<Assets<Image>>,
    mut commands: Commands,
    log: Option<Res<LogFilterConfig>>,
    // `--frame-stats` (see `super::frame_stats`): present only on a host asked
    // to measure, in which case the five phases below are stamped and recorded.
    mut stats: Option<ResMut<PaneFrameStats>>,
    // The diagnostic A/B toggles that go with it; absent means none.
    experiments: Option<Res<PaneExperiments>>,
) {
    let Some(mut host) = host else {
        return;
    };
    // The bus is OPTIONAL because the host-lobby surface (issue #1325) is not a
    // participant: `phoenix-host --client-dir dist --world <w>` with no --pane
    // has a pane host, one window and nothing on the pane bus at all.
    if let Some(bus) = &bus {
        // Panes closed since the last frame — by a fault below, by the operator,
        // or by anything else holding the bus — lose their view here, BEFORE
        // anything is pumped or drawn. A view left behind is not inert: the
        // input systems would still route to it, `pump_pane` would still drain a
        // live page's records into a registry that refuses them, once per frame,
        // for the rest of the run.
        retire_closed_panes(&mut host, bus, &mut commands, &log);
        // Panes the bus opened without a view get one here, BEFORE the frame
        // drives them: a pane recreated after a fault (issue #1125), so it
        // reloads its console and reconnects — the in-process analogue of a
        // phone redialling — and a station console the lobby's screen row just
        // opened (issue #1331), so it loads the client page and joins.
        open_pending_views(
            &mut host,
            bus,
            stations.as_deref(),
            bridge.as_ref().map(|b| &b.layout),
            &mut images,
            &mut commands,
            &log,
        );
    }
    // Presentation time, never simulation time: an unmeasured host reads no
    // clock here at all, and a measured one stamps nothing the simulation can
    // observe — see the module note in `super::frame_stats`.
    let clock = stats.is_some();
    let stamp = |on: bool| on.then(Instant::now);
    let elapsed_ms =
        |from: Option<Instant>| from.map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
    let experiments = experiments.as_deref().copied().unwrap_or_default();

    let phase = stamp(clock);
    host.runtime.update();
    let update_ms = elapsed_ms(phase);

    let phase = stamp(clock);
    let mut pushed_this_frame = vec![false; host.windows.len()];
    for (index, pane) in host.windows.iter_mut().enumerate() {
        // A load that has finished is what makes pushes legal. Asking the view
        // each frame (rather than trusting a callback) keeps this to one place.
        let was_loaded = pane.surface.is_ready();
        if pane.surface.refresh_loaded() && !was_loaded {
            crate::pinfo!(
                log,
                LogCat::Lobby,
                "pane host: {} finished loading",
                if pane.is_hud_overlay() {
                    "the viewscreen HUD".to_string()
                } else if pane.is_host_lobby() {
                    "the host lobby".to_string()
                } else {
                    format!("{} console", pane.id)
                }
            );
        }
        // The HUD overlay (issue #422, native port) is driven by the host's
        // `HudStateChanged`, cached in `ViewscreenHudLatest` so a state that
        // arrived before the page loaded still reaches it. Push the newest each
        // frame it is drawn — one idempotent `__updateHud` on an already-
        // repainting transparent surface, evaluated here so the DOM change is
        // picked up by the render below.
        if pane.is_hud_overlay() {
            if let (Some(json), true) = (&hud_latest.json, pane.surface.is_ready()) {
                let arg = serde_json::to_string(json).unwrap_or_else(|_| "\"{}\"".into());
                if pane
                    .surface
                    .push(&format!("window.__updateHud({arg})"))
                    .is_ok()
                {
                    pushed_this_frame[index] = true;
                }
            }
            continue;
        }
        // The lobby surface rides its OWN bridge, over the same `PaneSurface`.
        // Nothing it says is a `ClientMessage` and nothing it hears is a
        // projection, so nothing it says may reach the pane bus — which is the
        // whole reason the two are separate.
        if pane.is_host_lobby() {
            if let Some(lobby) = &lobby {
                let report = pump_host_lobby(&lobby.0, &mut pane.surface);
                pushed_this_frame[index] = report.pushed > 0;
                if let Some(failure) = &report.push_failure {
                    // Ordinary in the window between "the document loaded" and
                    // "its module island ran" — the state is kept and retried,
                    // so this is a debug line rather than a warning.
                    crate::pdebug!(
                        log,
                        LogCat::Lobby,
                        "pane host: the host lobby deferred a push: {failure}"
                    );
                }
            }
            continue;
        }
        let Some(bus) = &bus else { continue };
        let report = pump_pane(&bus.0, pane.id, &mut pane.surface);
        pushed_this_frame[index] = report.pushed > 0;
        for refusal in &report.refusals {
            crate::pwarn!(log, LogCat::Admit, "pane host: {}: {refusal}", pane.id);
        }
    }

    let pump_ms = elapsed_ms(phase);

    let phase = stamp(clock);
    host.runtime.render();
    let render_ms = elapsed_ms(phase);

    let phase = stamp(clock);
    let mut copy_ms = 0.0;
    let mut copied = 0usize;
    let mut forced = 0usize;
    let mut pixels = 0u64;
    for (index, pane) in host.windows.iter_mut().enumerate() {
        // `untracked` (frame experiment): fetch without telling Bevy the asset
        // changed, and say so below only when something was actually copied.
        // A tracked `get_mut` re-creates the pane's GPU texture in full
        // whether or not a pixel moved.
        let image = if experiments.untracked {
            images.get_mut_untracked(&pane.image)
        } else {
            images.get_mut(&pane.image)
        };
        let Some(image) = image else {
            continue;
        };
        let Some(data) = image.data.as_mut() else {
            continue;
        };
        // A push we just made is trusted on its own regardless of what the
        // surface reports: a plain attribute write is real DOM state that
        // changed and Ultralight's dirty-bounds tracking does not always flag
        // it. The `noforce` frame experiment switches that trust off to
        // measure what it costs.
        let force = pushed_this_frame[index] && !experiments.noforce;
        let copy_started = stamp(clock);
        let outcome = pane.surface.view.copy_frame(data, force);
        copy_ms += elapsed_ms(copy_started);
        let mut painted = false;
        match outcome {
            Ok(rect) => {
                pane.copy_failures = 0;
                if let Some(rect) = rect {
                    painted = true;
                    copied += 1;
                    if force {
                        forced += 1;
                    }
                    pixels += rect.pixel_count();
                }
            }
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
                //
                // The lobby surface has no such path and must not be given one:
                // it holds no station to fall back to AI control, and it is
                // PERMANENT (issue #1325) — closing it would be the one thing
                // every later slice is told it may assume never happens. A dead
                // lobby view is a warning per frame and a blank surface, which
                // is honest and recoverable by restarting the host. The HUD
                // overlay (issue #422, native port) is the same: no station, no
                // bus entry, so it cannot ride the Backfill path either.
                if pane.copy_failures >= VIEW_CRASH_COPY_FAILURES
                    && !pane.is_host_lobby()
                    && !pane.is_hud_overlay()
                {
                    if let Some(bus) = &bus {
                        bus.0.fault(pane.id, PaneFault::ViewCrashed);
                    }
                }
            }
        }
        if experiments.untracked && painted {
            // Now Bevy is told: this is the `Modified` the untracked fetch
            // above withheld, issued only for a frame that copied something.
            let _ = images.get_mut(&pane.image);
        }
    }
    let publish_ms = (elapsed_ms(phase) - copy_ms).max(0.0);
    if let Some(stats) = stats.as_mut() {
        stats.record_pane(PaneFrameSample {
            update_ms,
            pump_ms,
            render_ms,
            copy_ms,
            publish_ms,
            panes: host.windows.len(),
            copied,
            forced,
            pixels,
        });
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
    let Some(bus) = &bus else { return };
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

/// Build an Ultralight view for every pane the bus opened without one — a pane
/// recreated after a fault (issue #1125), and a station console the lobby's
/// screen row just opened (issue #1331).
///
/// **Where the view goes is [`home_for_pane`]'s decision, not this function's**
/// (issue #1333). That rule — the live Station slot first, then a refusal for a
/// console the law seats but the adapter cannot place, then the stored primary
/// tile, and whether a refusal is *retried* — is pure and CI-tested in
/// [`super::placement`], because "a crashed
/// console reopens on its own monitor, never over the viewscreen" is a claim,
/// and a claim only provable on a Windows machine with a GPU is a claim nobody
/// checks. What is left here is the Ultralight half: mint the Station camera,
/// build the view, and say what happened.
fn open_pending_views(
    host: &mut PaneHost,
    bus: &PaneBusResource,
    stations: Option<&BridgeStationSurfaces>,
    bridge: Option<&BridgeLayout>,
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
        // A camera this pane's build MINTED, as opposed to one it joined. Only
        // the minted one is this build's to clean up if the view then fails —
        // see the `Err` arm below.
        let mut minted_camera: Option<Entity> = None;
        let placement = match home_for_pane(&name, stations, bridge, &host.tiles) {
            PaneHome::Station {
                window,
                origin,
                size,
                scale,
                window_origin,
            } => PaneSeat {
                // One 2-D camera per Station window, reused as consoles come and
                // go on it — the same arrangement `init_pane_host` makes, and
                // the same list `retire_closed_panes` despawns from when a
                // Station window's last console closes.
                station_camera: Some({
                    let (camera, minted) = station_camera(host, commands, window);
                    if minted {
                        minted_camera = Some(camera);
                    }
                    camera
                }),
                window,
                origin,
                size,
                scale,
                window_origin,
            },
            PaneHome::PrimaryTile { origin, size } => PaneSeat {
                window: host.primary_window,
                station_camera: None,
                origin,
                size,
                scale: host.scale,
                window_origin: (0, 0),
            },
            PaneHome::Nowhere(reason) => {
                // Not built, and — for a reason a rebuild could fix — FAULTED
                // rather than dropped. `take_pending_views` has already drained
                // this entry, so a bare `continue` is final: the bus still lists
                // the pane as open and nothing is left to build a view for it.
                // For a SEATED console that is the same "worst of both" the
                // `Err` arm below describes, and it is reachable in a single
                // frame — `reconcile_seated_consoles` reaches its grace, closes
                // and recreates the console (resetting its own strike counter),
                // this drain drops the entry while the slot is still missing,
                // and the next frame the slot returns, so its health check
                // (pane open AND slot present) reads healthy for ever over a
                // black screen. The fault rides #1125's bounded path instead,
                // ending in a rebuild that lands or in the seat being given back
                // with a notice on the row.
                //
                // WHICH reasons retry is `NoHome::should_retry`'s to say, not
                // this function's: everything here is behind
                // `--features ultralight`, which no CI job compiles, and a
                // fault-or-skip choice made here is a choice nothing checks —
                // the whole reason `super::placement` exists (issue #1333).
                let retry = reason.should_retry();
                let outcome = if retry {
                    "faulting it, so the rebuild is retried on the same identity and its seat \
                     is repaired or honestly given back"
                } else {
                    "leaving it as it is: a retry would have nowhere to aim, so the pane stays \
                     open with no view and nothing else picks it up"
                };
                crate::pwarn!(
                    log,
                    LogCat::Lobby,
                    "pane host: {new_id} ({name}) is not built: {} — {outcome}",
                    reason.reason()
                );
                if retry {
                    bus.0.fault(new_id, PaneFault::ViewCrashed);
                }
                continue;
            }
        };
        let on_station = placement.station_camera.is_some();
        match make_pane_view(&host.runtime, images, commands, new_id, &url, placement) {
            Ok(window) => {
                host.windows.push(window);
                recreated_any = true;
                crate::pinfo!(
                    log,
                    LogCat::Lobby,
                    "pane host: {new_id} ({name}) is showing its console on {} — loading the \
                     client page, from which it joins and claims like any phone",
                    if on_station {
                        "its Station window"
                    } else {
                        "the viewscreen window"
                    }
                );
            }
            Err(e) => {
                // The camera is created BEFORE the fallible build, because
                // `make_pane_view` needs it to target the canvas it makes. So a
                // failed build must take it back: `retire_closed_panes` only
                // sweeps cameras when some pane CLOSES, and this pane never
                // opened a window at all — the camera would sit there clearing a
                // Station window to black for the life of the process, and the
                // next attempt on that window would join it rather than notice.
                if let Some(camera) = minted_camera {
                    commands.entity(camera).try_despawn();
                    host.station_cameras.retain(|(_, c)| *c != camera);
                }
                crate::pwarn!(
                    log,
                    LogCat::Lobby,
                    "pane host: could not build the view for {new_id} ({name}): {e}"
                );
                // And the pane is FAULTED rather than left open with nothing
                // behind it (issue #1331). A pane the bus lists as open but that
                // has no view is the worst of both: the station reads as claimed,
                // the screen is black, and nothing retries. `ViewCrashed` is
                // exactly what this is — a view that will not answer — so it
                // rides #1125's own path: close (one honest `PlayerDisconnected`,
                // the station on `Backfill`), then a bounded rebuild on the same
                // token. When that budget is spent the pane stays closed, and
                // `bridge_display::reconcile_seated_consoles` gives the seat back
                // so the operator's row stops claiming a screen that is black.
                bus.0.fault(new_id, PaneFault::ViewCrashed);
            }
        }
    }
    // A recreated pane is a new surface on the same identity: rebuild the router
    // and reconcile the focus order so #1124's input routing reaches it.
    if recreated_any {
        host.rebuild_layout();
    }
}

/// Where one pane's view is built: which window, which rectangle on it, and
/// which camera draws it.
///
/// A struct rather than six more parameters, and the six are exactly the fields
/// [`PaneWindow`] needs to be told: they travel together everywhere, and a
/// positional `(u32, u32)` pair next to another `(u32, u32)` pair is the shape
/// of an argument-order bug nothing would catch.
///
/// Named for the seat rather than the placement, because
/// [`PanePlacement`](crate::native_host::input_routing::PanePlacement) is the
/// input router's own type for where a pane sits in *screen* coordinates. This
/// one is how the view is BUILT; that one is how a click is resolved.
struct PaneSeat {
    /// The OS window this pane renders on — the primary (viewscreen) window for
    /// a tiled pane, a Station window for a composited one.
    window: Entity,
    /// The 2-D camera drawing it onto a Station window. `None` for a pane on the
    /// primary window, which uses the game's own default UI camera.
    station_camera: Option<Entity>,
    origin: (u32, u32),
    size: (u32, u32),
    scale: f64,
    /// The window's top-left on the virtual desktop, physical pixels — `(0, 0)`
    /// for the primary window. What makes the input router resolve a click in
    /// the monitor's own coordinate space.
    window_origin: (i32, i32),
}

/// The 2-D camera rendering `window`'s panes, spawned if this is the first one.
///
/// One camera per Station window, reused as consoles come and go on it — the
/// same arrangement `init_pane_host` makes at boot, and the same
/// `station_cameras` list `retire_closed_panes` despawns from when a Station
/// window's last console closes.
///
/// Returns whether this call **minted** the camera, because the caller builds a
/// view that can fail afterwards and only a camera it minted is its to take
/// back — despawning one it merely joined would blank every console already on
/// that window.
fn station_camera(host: &mut PaneHost, commands: &mut Commands, window: Entity) -> (Entity, bool) {
    if let Some((_, camera)) = host.station_cameras.iter().find(|(w, _)| *w == window) {
        return (*camera, false);
    }
    let camera = commands
        .spawn((
            Camera2d,
            Camera {
                order: 0,
                clear_color: ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
            RenderTarget::Window(WindowRef::Entity(window)),
            CameraRenderGraph::new(Core2d),
        ))
        .id();
    host.station_cameras.push((window, camera));
    (camera, true)
}

/// Create one pane's Ultralight view, its texture and its on-screen canvas at
/// `placement` — the path every pane built **after** init takes (issues #1125,
/// #1331).
///
/// The same construction `init_pane_host` does inline, but against
/// `Assets<Image>`/`Commands` rather than an exclusive `&mut World`, because
/// `drive_panes` is an ordinary system. A creation or load failure here fails
/// only THIS pane — its station simply stays on Backfill — rather than the whole
/// host, which is right for both a recovery and a console the operator can
/// simply close and re-open.
fn make_pane_view(
    runtime: &UltralightRuntime,
    images: &mut Assets<Image>,
    commands: &mut Commands,
    id: PaneId,
    url: &str,
    placement: PaneSeat,
) -> Result<PaneWindow, PaneSurfaceError> {
    let PaneSeat {
        window,
        station_camera,
        origin,
        size,
        scale,
        window_origin,
    } = placement;
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
    let mut canvas = commands.spawn((
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
    // A composited pane's canvas renders on its Station camera; a tiled one uses
    // the default UI camera and takes no marker — as at init.
    if let Some(cam) = station_camera {
        canvas.insert(UiTargetCamera(cam));
    }
    let canvas = canvas.id();
    Ok(PaneWindow {
        id,
        surface,
        image: handle,
        canvas,
        window,
        origin,
        size,
        scale,
        window_origin,
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
    // The host-lobby surface (issue #1325) and the viewscreen HUD overlay (issue
    // #422, native port) are never in `open_pane_ids` — neither is a participant
    // and neither has a registry entry — and both are PERMANENT, composited for
    // the life of the host, so neither is a candidate for retirement in either
    // test below. Missing the HUD here retired it the instant the bus went active
    // at InProgress, which is exactly when its frame should appear.
    let survives = |w: &PaneWindow| open.contains(&w.id) || w.is_host_lobby() || w.is_hud_overlay();
    if host.windows.iter().all(survives) {
        return;
    }
    let mut closed: Vec<PaneId> = Vec::new();
    host.windows.retain(|window| {
        if survives(window) {
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
    // that no longer exists.
    for pane in &closed {
        host.contacts.release_pane(*pane);
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
    // one participant's focus onto another.
    host.rebuild_layout();
}
