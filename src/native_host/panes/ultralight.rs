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
//! # The pane thread (issue #1404)
//!
//! Bevy owns the canvases and input router in the ordinary Send resource
//! [`PaneHost`]. [`super::pane_thread::spawn_pane_thread`] constructs the
//! Ultralight runtime and all views on "phoenix-panes", and drives them there.
//! Input, creation, resize, visibility and scripts cross as [`PaneCommand`]s.
//! [`drive_pane_host`] drains the replies and turns current-epoch frames into
//! persistent texture uploads; it never calls the SDK.
//!
//! The renderer's factory is Send, while its runtime and views are not. Startup
//! waits for Started before building seats. Per-seat creation errors take the
//! console's bounded recovery path; renderer death is terminal and closes every
//! console without respawning the thread. The simulation continues in unwind
//! builds; a release panic aborts the process as configured in Cargo.toml.
//!
//! Hidden lobby and HUD views keep receiving state but stop copying frames.
//! Host-lobby actions now round-trip one pane period plus a Bevy frame.
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

use std::sync::mpsc::TryRecvError;
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
use super::hud::{hud_z_index, LOBBY_Z_INDEX};
use super::mirror::{MirrorPane, PaneMirror, VIEW_CRASH_COPY_FAILURES};
use super::pane_thread::{
    spawn_pane_thread, FrameRect, PaneCommand, PaneEvent, PaneInput, PaneKeyCode, PaneKind,
    PaneRuntime, PaneSpecOwned, PaneThreadConfig, PaneThreadHandle, PaneView,
};
use super::placement::{home_for_pane, PaneHome, PaneTile};
use super::recovery::{close_after_thread_failure, service_faults, PaneFault};
use super::registry::PaneId;
use super::render_geometry::PaneRenderScale;
use super::surface::{PaneSurface, PaneSurfaceError};
use super::surface_stats::{elapsed_ns, DiscardReason, Operation, SurfaceObserver};
use super::upload::{PanePendingUploads, PaneUpload};
use super::PaneBusResource;
use crate::console_bridge::HudStateChanged;
use crate::core::messages::GamePhase;
use crate::logging::{LogCat, LogFilterConfig};
use crate::native_host::bridge_display::{BridgeLayoutResource, BridgeStationSurfaces};
use crate::native_host::bridge_layout::BridgeLayout;
use crate::native_host::bridge_profile::PaneRect;
use crate::native_host::host_lobby::{
    host_lobby_drain_script, HostLobbyBridgeResource, HostLobbyRevealResource,
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

/// The latest HUD state and encoded command, cached by source revision. The
/// worker retains the command until each loaded view has applied it and for
/// reapplication after load, reveal or resize.
#[derive(Resource, Debug, Default)]
struct ViewscreenHudLatest(super::hud::HudScriptCache);

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
    /// Whether this page composites over what is behind it (issue #1404).
    ///
    /// Set at construction from the surface's [`PaneKind`], the same predicate
    /// the texture format and the fill are minted from, and read by exactly one
    /// thing: which copy [`PaneView::copy_frame`] makes. Before this the copy
    /// loop asked the *pane window* whether it was the HUD, so the one fact that
    /// has to agree with the texture — straight alpha or verbatim BGRA — was
    /// decided a level above the thing that knows it. A surface now carries its
    /// own answer, which is what lets a copy happen anywhere the surface is.
    transparent: bool,
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
        Self::with_transparency(view, false)
    }

    /// Wrap a freshly created pane view whose page is **transparent** — the
    /// viewscreen HUD overlay (issue #422, native port), and nothing else today.
    ///
    /// A separate constructor rather than a parameter on [`new`](Self::new)
    /// because `new` is what the ignored SDK integration tests call, and every
    /// caller that is not the HUD wants the opaque answer. See
    /// [`transparent`](Self::transparent) for what the flag decides.
    pub fn with_transparency(view: UltralightPane, transparent: bool) -> Self {
        Self {
            view,
            loaded: false,
            transparent,
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
            // Opaque chrome, like a console: `PaneKind::Lobby.transparent()` is
            // false, and this is the same answer said in the adapter.
            transparent: false,
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

/// The frame half of the seam (issue #1404, slice 2).
///
/// Every call here is one the frame loop used to make by reaching into
/// `surface.view`. Going through the trait instead is what lets the loop be
/// written — and tested, against
/// [`pane_thread::doubles`](super::pane_thread) — without an SDK, and what will
/// let it run on a thread of its own. The calls, and their order, are exactly
/// the calls and the order they were.
impl PaneView for UltralightPaneSurface {
    fn refresh_loaded(&mut self) -> bool {
        UltralightPaneSurface::refresh_loaded(self)
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.view.resize(width, height);
    }

    fn input(&mut self, input: &PaneInput) {
        match input {
            PaneInput::MouseMove { x, y } => self.view.mouse_move(*x, *y),
            PaneInput::MouseDown { x, y } => self.view.mouse_down(*x, *y, UlMouseButton::Left),
            PaneInput::MouseUp { x, y } => self.view.mouse_up(*x, *y, UlMouseButton::Left),
            PaneInput::Scroll { dx, dy } => self.view.scroll(*dx, *dy),
            // Every editing key is a raw key-down with native code 0 and no
            // modifiers, exactly as `forward_keyboard_text` has always sent them
            // — the modifiers gap noted there is unchanged by this seam.
            PaneInput::Key(code) => self.view.key(
                KeyEventType::RawKeyDown,
                match code {
                    PaneKeyCode::Back => VirtualKeyCode::Back,
                    PaneKeyCode::Return => VirtualKeyCode::Return,
                    PaneKeyCode::Left => VirtualKeyCode::Left,
                    PaneKeyCode::Right => VirtualKeyCode::Right,
                    PaneKeyCode::Up => VirtualKeyCode::Up,
                    PaneKeyCode::Down => VirtualKeyCode::Down,
                    PaneKeyCode::Home => VirtualKeyCode::Home,
                    PaneKeyCode::End => VirtualKeyCode::End,
                    PaneKeyCode::Delete => VirtualKeyCode::Delete,
                    PaneKeyCode::Tab => VirtualKeyCode::Tab,
                },
                0,
                Modifiers::default(),
            ),
            PaneInput::KeyChar(text) => self.view.key_char(text),
            PaneInput::Focus => self.view.focus(),
            PaneInput::Unfocus => self.view.unfocus(),
        }
    }

    fn copy_frame(
        &mut self,
        dst: &mut [u8],
        force: bool,
    ) -> Result<Option<FrameRect>, PaneSurfaceError> {
        // The transparent HUD takes the straight-alpha copy; every opaque
        // surface is moved verbatim into its BGRA texture — see
        // [`pane_texture_format`], which is minted from the same predicate.
        let copied = if self.transparent {
            self.view.copy_frame(dst, force)
        } else {
            self.view.copy_frame_bgra(dst, force)
        };
        copied
            .map(|rect| rect.map(FrameRect::from))
            .map_err(|e| PaneSurfaceError::Frame(e.to_string()))
    }
}

/// The one Ultralight renderer, behind the trait the pane loop mints views
/// through (issue #1404, slice 2).
///
/// A newtype rather than an `impl PaneRuntime for UltralightRuntime`, because
/// the mapping from a [`PaneKind`] to a `PaneSpec` — the transparency, the
/// per-pane ephemeral storage session, which drain script the surface gets — is
/// this host's policy and not vellum's. `!Send`, like the runtime it holds: what
/// crosses onto the pane thread is the closure that builds one.
pub struct UltralightHost {
    runtime: UltralightRuntime,
}

impl UltralightHost {
    /// Take ownership of a started runtime.
    pub fn new(runtime: UltralightRuntime) -> Self {
        Self { runtime }
    }
}

impl PaneRuntime for UltralightHost {
    type View = UltralightPaneSurface;

    fn update(&mut self) {
        self.runtime.update();
    }

    fn render(&mut self) {
        self.runtime.render();
    }

    fn create(
        &mut self,
        id: PaneId,
        kind: PaneKind,
        spec: &PaneSpecOwned,
        url: &str,
    ) -> Result<Self::View, PaneSurfaceError> {
        let spec = PaneSpec {
            width: spec.width,
            height: spec.height,
            device_scale: spec.device_scale,
            transparent: kind.transparent(),
            // One storage session per pane, named after the pane and never
            // written to disk — see the twin of this spec in `init_pane_host`
            // for what a shared session would cost.
            session: Some(PaneSession::ephemeral(id.to_string())),
        };
        let view = self
            .runtime
            .create_pane(&spec)
            .map_err(|e| PaneSurfaceError::Load(e.to_string()))?;
        // The lobby drains its OWN queue — the one place the two surfaces differ
        // below the URL.
        let mut surface = match kind {
            PaneKind::Lobby => UltralightPaneSurface::for_host_lobby(view),
            PaneKind::Console | PaneKind::Hud => {
                UltralightPaneSurface::with_transparency(view, kind.transparent())
            }
        };
        surface.load(url)?;
        Ok(surface)
    }
}

/// The texture a pane's pixels are copied into, by whether the page is
/// transparent (issue #1402).
///
/// An opaque page — every Station console, the lobby surface — is copied
/// **verbatim** from Ultralight's premultiplied-BGRA surface with
/// `copy_frame_bgra`, so its texture is `Bgra8UnormSrgb` and no byte is
/// swizzled or divided on the way. Only the transparent HUD overlay needs
/// straight alpha, and pays for it with `copy_frame` into `Rgba8UnormSrgb`.
/// Both `Image::new_fill` sites and the copy loop decide through this one
/// predicate, so the format and the copy cannot come apart.
const fn pane_texture_format(transparent: bool) -> TextureFormat {
    if transparent {
        TextureFormat::Rgba8UnormSrgb
    } else {
        TextureFormat::Bgra8UnormSrgb
    }
}

/// The colour a pane's texture is minted in, by the same predicate
/// [`pane_texture_format`] uses.
///
/// The transparent HUD overlay starts fully clear, so the frames before its
/// first copy show the 3-D scene rather than a black rectangle over it; every
/// opaque surface starts black. It matters more since issue #1404: the texture
/// is written in place and only where the page repainted, so this fill is what
/// shows anywhere a frame has not yet reached — including the whole surface for
/// the frame or two after a resize mints a new one.
const fn pane_fill(transparent: bool) -> [u8; 4] {
    if transparent {
        [0, 0, 0, 0]
    } else {
        [0, 0, 0, 255]
    }
}

/// The main world's part of a pane. SDK objects and staging pools live on the
/// pane thread; these assets and coordinates are safe to read in any Bevy system.
struct PaneCanvasData {
    image: Handle<Image>,
    canvas: Entity,
    window: Entity,
    origin: (u32, u32),
    size: (u32, u32),
    scale: f64,
    render_scale: PaneRenderScale,
    window_origin: (i32, i32),
    /// The camera targeted while creation is pending. Cleanup rechecks all
    /// live seats because another create may have joined it before a refusal.
    pending_camera: Option<Entity>,
}
type PaneWindow = MirrorPane<PaneCanvasData>;

impl PaneWindow {
    fn raster_size(&self) -> (u32, u32) {
        self.render_scale.geometry(self.size, self.scale).size
    }

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

/// Bevy's canvas and routing state, with a channel to the renderer's own thread.
#[derive(Resource)]
pub struct PaneHost {
    mirror: PaneMirror<PaneCanvasData>,
    console_render_scale: PaneRenderScale,
    thread: Option<PaneThreadHandle>,
    hud_last_sent_revision: u64,
    gamepads_last_sent: Option<String>,
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
    /// reader, because the routing systems hold a resource borrow of the host.
    pub fn router(&self) -> &PaneRouter {
        &self.router
    }

    /// The pane currently holding keyboard focus, if any — the observable half of
    /// [`traverse_focus_keys`] and pointer-follow focus.
    pub fn focused_pane(&self) -> Option<PaneId> {
        self.focus.focused()
    }

    pub fn thread_is_running(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(PaneThreadHandle::is_running)
    }

    fn send(&self, cmd: PaneCommand) {
        if let Some(thread) = &self.thread {
            // Disconnection is handled once by the event drain.
            let _ = thread.send(cmd);
        }
    }

    /// Give a pane's view keyboard focus and take it from whatever held it,
    /// keeping Ultralight's single-focused-view invariant in step with the model.
    fn focus_view(&mut self, previous: Option<PaneId>, next: Option<PaneId>) {
        if previous == next {
            return;
        }
        // Unfocus BEFORE focus, and both through the input seam: they are two
        // messages in one FIFO, and a focus that arrived before the unfocus it
        // replaces would leave Ultralight's single-focused-view invariant
        // pointing at nothing.
        if let Some(id) = previous {
            self.send(PaneCommand::Input {
                id,
                input: PaneInput::Unfocus,
            });
        }
        if let Some(id) = next {
            self.send(PaneCommand::Input {
                id,
                input: PaneInput::Focus,
            });
        }
    }

    /// Rebuild the router and reconcile the focus order and touch captures after
    /// the set of open panes changed.
    fn rebuild_layout(&mut self) {
        self.router = build_router(&self.mirror, self.lobby_present);
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
fn build_router(windows: &PaneMirror<PaneCanvasData>, lobby_present: bool) -> PaneRouter {
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

#[derive(Resource)]
struct PaneHostStarting(PaneThreadHandle);

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
        app.add_systems(PostUpdate, stop_pane_host_on_exit);
        // Idempotent, and belt-and-braces: `PaneUploadPlugin` (registered
        // unconditionally in `native_host::app`) already puts this in, but
        // `drive_pane_host` takes it as a plain `ResMut` and the ignored SDK tests
        // build their own apps around this plugin alone.
        app.init_resource::<PanePendingUploads>();
        app.add_systems(PreUpdate, init_pane_host).add_systems(
            Update,
            (
                // Before the input group: whether the lobby surface is placed in
                // the router at all is decided here, and a click this frame must
                // be routed against this frame's answer.
                sync_host_lobby_presence,
                // Also before input and before the frame copy: a resize moves the
                // views and re-tiles, and both the router and `drive_pane_host` must
                // see this frame's rects. Gated like `drive_pane_host` — no image
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
                // state (issue #422, native port) — both BEFORE `drive_pane_host`, so
                // the frame it draws this tick shows the right presence and the
                // newest readout. `cache_hud_state` is gated on the host actually
                // registering `HudStateChanged`, so the rendererless Contract test
                // (which has no server plugins) stands the pane host up without it.
                sync_viewscreen_hud_presence,
                cache_hud_state
                    .run_if(resource_exists::<bevy::ecs::message::Messages<HudStateChanged>>),
                // Feed the host's gamepads into each console pane (native gamepad
                // route) BEFORE `drive_pane_host`, so the snapshot the page polls this
                // tick is current. Gated on the input plugin so the rendererless
                // Contract host — which has neither input nor gilrs — skips it.
                push_gamepads_to_panes.run_if(resource_exists::<ButtonInput<MouseButton>>),
                drive_pane_host.run_if(resource_exists::<Assets<Image>>),
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
    if world.get_resource::<PaneHost>().is_some()
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

    // The `plog!` family, like `drive_pane_host` below: an exclusive system can
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
    // The bus itself is read per frame by `drive_pane_host`; what matters here is
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

    // Phase one starts only after a real primary window and profile surfaces
    // exist. Phase two waits for the explicit Started handshake.
    if world.get_resource::<PaneHostStarting>().is_none() {
        let config = PaneThreadConfig {
            bus: world.get_resource::<PaneBusResource>().map(|b| b.0.clone()),
            lobby: world
                .get_resource::<HostLobbyBridgeResource>()
                .map(|l| l.0.clone()),
            measure: world.contains_resource::<PaneFrameStats>(),
            observer: world.get_resource::<SurfaceObserver>().cloned(),
            on_shutdown_timeout: Some(report_pane_shutdown_timeout),
            ..Default::default()
        };
        match spawn_pane_thread(config, || {
            UltralightRuntime::start(&RuntimeOptions::default())
                .map(UltralightHost::new)
                .map_err(|e| e.to_string())
        }) {
            Ok(thread) => {
                world.insert_resource(PaneHostStarting(thread));
            }
            Err(error) => {
                crate::perror!(
                    log,
                    LogCat::Lobby,
                    "pane host: cannot start pane thread: {error}"
                );
                if let Some(bus) = world.get_resource::<PaneBusResource>() {
                    close_after_thread_failure(&bus.0);
                }
                world.insert_resource(PaneHostFailed);
            }
        }
        return;
    }
    match world.resource::<PaneHostStarting>().0.try_recv() {
        Ok(PaneEvent::Started(Ok(()))) => {}
        Err(TryRecvError::Empty) => return,
        result => {
            crate::perror!(
                log,
                LogCat::Lobby,
                "pane host: renderer startup failed: {result:?}"
            );
            world.remove_resource::<PaneHostStarting>();
            if let Some(bus) = world.get_resource::<PaneBusResource>() {
                close_after_thread_failure(&bus.0);
            }
            world.insert_resource(PaneHostFailed);
            return;
        }
    }
    let thread = world.remove_resource::<PaneHostStarting>().unwrap().0;

    // The bus, for resolving each pane's participant name into the layout that a
    // recreated pane (issue #1125) rebuilds against. Absent on a lobby-only
    // host, which has no participants to resolve.
    let bus = world.get_resource::<PaneBusResource>().cloned();

    let mut windows = PaneMirror::new();
    let experiments = world
        .get_resource::<PaneExperiments>()
        .copied()
        .unwrap_or_default();
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

        let kind = if seat.hud {
            PaneKind::Hud
        } else if seat.lobby {
            PaneKind::Lobby
        } else {
            PaneKind::Console
        };
        let visible = !(seat.hud || seat.lobby && !lobby_present);
        let render_scale = experiments.render_scale(kind);
        let raster = render_scale.geometry(seat.size, seat.scale);
        let _ = thread.send(PaneCommand::Create {
            id,
            kind,
            spec: PaneSpecOwned {
                width: raster.size.0,
                height: raster.size.1,
                device_scale: raster.device_scale,
            },
            url: url.to_owned(),
            epoch: 0,
            visible,
        });
        // The transparent HUD overlay starts fully clear so the one frame before
        // its first copy shows the 3-D scene, not a black fill; opaque surfaces
        // start black — see `pane_fill`.
        let image = Image::new_fill(
            Extent3d {
                width: raster.size.0,
                height: raster.size.1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &pane_fill(seat.hud),
            pane_texture_format(seat.hud),
            // RENDER_WORLD only (issue #1404): the data is moved out at the
            // first extract and this asset is never written from the main world
            // again. Every frame after it is a `write_texture` into the texture
            // this created — see `super::upload`. A stray `get_mut` on a pane
            // image would now be an `AlreadyExtracted` error rather than a
            // silent per-frame texture re-creation, which is the right way
            // round.
            RenderAssetUsages::RENDER_WORLD,
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
            world.entity_mut(canvas).insert(ZIndex(LOBBY_Z_INDEX));
        }
        // The HUD normally covers scene UI, but the F9-revealed lobby must be
        // above its border: the Settings cog and popup are inside that texture.
        // This also preserves the lobby's existing order below tiled consoles.
        if seat.hud {
            world
                .entity_mut(canvas)
                .insert(ZIndex(hud_z_index(lobby_present)));
        }
        windows.insert(
            id,
            kind,
            visible,
            PaneCanvasData {
                image: handle,
                canvas,
                window: seat.window,
                origin: seat.origin,
                size: seat.size,
                scale: seat.scale,
                render_scale,
                window_origin: seat.window_origin,
                pending_camera: station_camera,
            },
        );
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
        let _ = thread.send(PaneCommand::Input {
            id: first,
            input: PaneInput::Focus,
        });
    }
    world.insert_resource(PaneHost {
        mirror: windows,
        console_render_scale: experiments.render_scale(PaneKind::Console),
        thread: Some(thread),
        hud_last_sent_revision: 0,
        gamepads_last_sent: None,
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
    host: Option<ResMut<PaneHost>>,
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
        .mirror
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
    if let Some(pane) = host.mirror.get_mut(HOST_LOBBY_SURFACE_ID) {
        pane.visible = present;
    }
    host.send(PaneCommand::SetVisible {
        id: HOST_LOBBY_SURFACE_ID,
        visible: present,
    });
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
/// condition, red alert). Encode only a changed value; [`drive_pane_host`] sends
/// each revision to the worker's retained slot. A state that arrives before the
/// surface is ready therefore still reaches its first loaded frame.
fn cache_hud_state(
    mut latest: ResMut<ViewscreenHudLatest>,
    mut events: MessageReader<HudStateChanged>,
) {
    if let Some(event) = events.read().last() {
        latest.0.update(&event.json);
    }
}

/// Show the viewscreen HUD in `InProgress` and `GameOver`, hiding it otherwise.
///
/// F9 can reveal the lobby over either live phase. Both views remain composited,
/// but the passive HUD then goes beneath the lobby's interactive controls. This
/// follows `sync_host_lobby_presence` in the same chain, so draw and input use
/// the same frame's reveal decision. Hiding chrome restores the normal HUD layer.
fn sync_viewscreen_hud_presence(
    host: Option<ResMut<PaneHost>>,
    phase: Option<Res<State<GamePhase>>>,
    mut nodes: Query<(&mut Node, &mut ZIndex)>,
) {
    let (Some(mut host), Some(phase)) = (host, phase) else {
        return;
    };
    let Some(canvas) = host
        .mirror
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
    let visible = want != Display::None;
    if let Some(pane) = host.mirror.get_mut(VIEWSCREEN_HUD_SURFACE_ID) {
        if pane.visible != visible {
            pane.visible = visible;
            host.send(PaneCommand::SetVisible {
                id: VIEWSCREEN_HUD_SURFACE_ID,
                visible,
            });
        }
    }
    if let Ok((mut node, mut z_index)) = nodes.get_mut(canvas) {
        // Write only on a real change so an unchanged phase does not dirty the UI
        // layout every frame.
        if node.display != want {
            node.display = want;
        }
        let layer = hud_z_index(host.lobby_present);
        if z_index.0 != layer {
            z_index.0 = layer;
        }
    }
}

/// Stable browser-slot numbering for the host's gamepads (native gamepad route).
///
/// The page's per-station selection setting stores a slot index, so a slot must
/// stay pointed at the same physical pad for the session: a `gilrs` entity keeps
/// the slot it was first seen on. `had_any` records whether a pad has ever
/// existed, so unplug can remain held until the pane thread observes it.
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
/// consoles and install no receiver.
///
/// This fills the latest snapshot slot on changes. The pane thread pushes it
/// before Renderer::update, where the page's animation callbacks poll it.
fn push_gamepads_to_panes(
    host: Option<ResMut<PaneHost>>,
    pads: Query<(Entity, &Gamepad)>,
    mut slots: Local<GamepadSlots>,
) {
    let Some(mut host) = host else {
        return;
    };
    let readings = read_pads(&pads, &mut slots);
    let Some(script) = super::gamepad::held_gamepad_script(&readings, &mut slots.had_any) else {
        return;
    };
    if host.gamepads_last_sent.as_ref() != Some(&script) {
        host.send(PaneCommand::SetGamepadScript(Some(script.clone())));
        host.gamepads_last_sent = Some(script);
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
    host: Option<ResMut<PaneHost>>,
    windows: Query<&Window>,
    mut images: ResMut<Assets<Image>>,
    mut nodes: Query<&mut Node>,
    // The canvas's image handle is swapped here too (issue #1404): a resize
    // mints a NEW asset rather than resizing the old one, so the node has to be
    // pointed at it.
    mut canvases: Query<&mut ImageNode>,
) {
    let Some(mut host) = host else {
        return;
    };
    // Group pane indices by the window they sit on, preserving order — a pane's
    // tile index within its window is its position here, exactly as at init.
    let mut groups: Vec<(Entity, Vec<usize>)> = Vec::new();
    for (i, pane) in host.mirror.iter().enumerate() {
        match groups.iter_mut().find(|(w, _)| *w == pane.window) {
            Some((_, idxs)) => idxs.push(i),
            None => groups.push((pane.window, vec![i])),
        }
    }
    // Target (origin, size) per pane in physical pixels within its window.
    let mut targets: Vec<Option<((u32, u32), (u32, u32))>> = vec![None; host.mirror.len()];
    for (window_entity, idxs) in &groups {
        let Ok(window) = windows.get(*window_entity) else {
            continue;
        };
        let pw = window.physical_width().max(1);
        let ph = window.physical_height().max(1);
        // The host-lobby surface (issue #1325) and the HUD overlay (issue #422,
        // native port) are FULL-WINDOW overlays, not tiled consoles: each fills
        // the whole window and they stack by ZIndex (revealed lobby above HUD).
        // Only genuine console panes tile
        // among themselves — counting the overlays in the tile split is what
        // squeezed the HUD into half the viewscreen.
        let tiled: Vec<usize> = idxs
            .iter()
            .copied()
            .filter(|&i| !host.mirror[i].is_host_lobby() && !host.mirror[i].is_hud_overlay())
            .collect();
        for &i in idxs {
            if host.mirror[i].is_host_lobby() || host.mirror[i].is_hud_overlay() {
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
    // Split the borrow: the pane windows and the command queue are two fields of
    // the host, and a resize touches both.
    let host = &mut *host;
    let mut resize_commands = Vec::new();
    for (i, pane) in host.mirror.iter_mut().enumerate() {
        let Some((origin, size)) = targets[i] else {
            continue;
        };
        if origin == pane.origin && size == pane.size {
            continue;
        }
        // A NEW asset, not `Image::resize` (issue #1404): the pane images are
        // `RenderAssetUsages::RENDER_WORLD`, so their `data` was moved out at
        // the first extract and resizing a `data: None` image would leave the
        // GPU texture at its old size — a silent disagreement that every later
        // upload would be refused for. Minting one instead gives the render
        // world a fresh texture at the new size, and the epoch bump is what
        // makes any frame still in flight against the old one recognisable.
        let transparent = pane.is_hud_overlay();
        let raster = pane.render_scale.geometry(size, pane.scale);
        let handle = images.add(Image::new_fill(
            Extent3d {
                width: raster.size.0,
                height: raster.size.1,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            &pane_fill(transparent),
            pane_texture_format(transparent),
            RenderAssetUsages::RENDER_WORLD,
        ));
        pane.image = handle.clone();
        pane.epoch += 1;
        // Move the view, and start the generation the new texture belongs to —
        // through the command seam, so the view and its texture agree about
        // which frames are about which surface. All three (view, asset, node)
        // must agree, or a copy writes a buffer of the wrong length into the
        // image the next frame.
        //
        // The new epoch is visible to the main world immediately. The thread
        // resizes before the later input commands in this ordered stream;
        // frames still arriving at the old epoch are discarded by the mirror.
        resize_commands.push(PaneCommand::Resize {
            id: pane.id,
            width: raster.size.0,
            height: raster.size.1,
            epoch: pane.epoch,
        });
        if let Ok(mut canvas) = canvases.get_mut(pane.canvas) {
            canvas.image = handle;
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
    for command in resize_commands {
        host.send(command);
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
    host: Option<ResMut<PaneHost>>,
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
            // Move then down, in that order and into the same pane's stream:
            // Ultralight decides what is under the pointer from the move.
            host.send(PaneCommand::Input {
                id: hit.pane,
                input: PaneInput::MouseMove {
                    x: hit.local_x,
                    y: hit.local_y,
                },
            });
            host.send(PaneCommand::Input {
                id: hit.pane,
                input: PaneInput::MouseDown {
                    x: hit.local_x,
                    y: hit.local_y,
                },
            });
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
                    host.send(PaneCommand::Input {
                        id: captured,
                        input: PaneInput::MouseMove { x: lx, y: ly },
                    });
                }
            }
        }
    } else if let Some((_, _, _, Some(hit))) = cursor {
        host.send(PaneCommand::Input {
            id: hit.pane,
            input: PaneInput::MouseMove {
                x: hit.local_x,
                y: hit.local_y,
            },
        });
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
            host.send(PaneCommand::Input {
                id: captured,
                input: PaneInput::MouseUp { x: lx, y: ly },
            });
        }
    }

    // Scroll goes to the pane under the cursor.
    if scroll.delta != Vec2::ZERO {
        if let Some((_, _, _, Some(hit))) = cursor {
            host.send(PaneCommand::Input {
                id: hit.pane,
                input: PaneInput::Scroll {
                    dx: scroll.delta.x as i32,
                    dy: scroll.delta.y as i32,
                },
            });
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
    host: Option<ResMut<PaneHost>>,
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
                        // The focus above, then move, then down — one pane's
                        // stream, in the order the page must see them, which is
                        // the order they are queued in.
                        host.send(PaneCommand::Input {
                            id: hit.pane,
                            input: PaneInput::MouseMove {
                                x: hit.local_x,
                                y: hit.local_y,
                            },
                        });
                        host.send(PaneCommand::Input {
                            id: hit.pane,
                            input: PaneInput::MouseDown {
                                x: hit.local_x,
                                y: hit.local_y,
                            },
                        });
                    }
                }
            }
            TouchPhase::Moved => {
                if let Some(pane) = host.contacts.pane_for(touch.id) {
                    if let Some((lx, ly)) = host.router.project_into_pane(pane, phys.0, phys.1) {
                        host.send(PaneCommand::Input {
                            id: pane,
                            input: PaneInput::MouseMove { x: lx, y: ly },
                        });
                    }
                }
            }
            TouchPhase::Ended | TouchPhase::Canceled => {
                if let Some(pane) = host.contacts.end(touch.id) {
                    if let Some((lx, ly)) = host.router.project_into_pane(pane, phys.0, phys.1) {
                        host.send(PaneCommand::Input {
                            id: pane,
                            input: PaneInput::MouseUp { x: lx, y: ly },
                        });
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
fn traverse_focus_keys(host: Option<ResMut<PaneHost>>, keys: Res<ButtonInput<KeyCode>>) {
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
    host: Option<Res<PaneHost>>,
    keycodes: Res<ButtonInput<KeyCode>>,
    mut keys: MessageReader<KeyboardInput>,
) {
    // This runs after focus routing. Sending is a shared borrow, and its input
    // stream stays ordered behind the focus command that selected this pane.
    let Some(host) = host else {
        return;
    };
    let ctrl = keycodes.pressed(KeyCode::ControlLeft) || keycodes.pressed(KeyCode::ControlRight);
    let focused = host.focus.focused();
    for key in keys.read() {
        if !key.state.is_pressed() {
            continue;
        }
        let Some(pane) = focused else {
            continue;
        };
        // `key_char` is what actually puts a character into a field; the
        // editing keys below are raw key-downs. Both shapes are the protocol's,
        // and the adapter turns them back into the same view calls this made
        // directly before (issue #1404).
        let send = |input: PaneInput| host.send(PaneCommand::Input { id: pane, input });
        match &key.logical_key {
            Key::Character(text) => send(PaneInput::KeyChar(text.to_string())),
            Key::Space => send(PaneInput::KeyChar(" ".to_string())),
            Key::Backspace => send(PaneInput::Key(PaneKeyCode::Back)),
            Key::Enter => send(PaneInput::Key(PaneKeyCode::Return)),
            // Caret movement and forward-delete: without these the caret cannot
            // move within a field and forward-delete is unavailable, so a comms
            // reply or a waypoint name can only be typed and back-spaced. Each
            // maps cleanly to an Ultralight virtual key and is forwarded as a raw
            // key-down like the arms above (issue #1124).
            Key::ArrowLeft => send(PaneInput::Key(PaneKeyCode::Left)),
            Key::ArrowRight => send(PaneInput::Key(PaneKeyCode::Right)),
            Key::ArrowUp => send(PaneInput::Key(PaneKeyCode::Up)),
            Key::ArrowDown => send(PaneInput::Key(PaneKeyCode::Down)),
            Key::Home => send(PaneInput::Key(PaneKeyCode::Home)),
            Key::End => send(PaneInput::Key(PaneKeyCode::End)),
            Key::Delete => send(PaneInput::Key(PaneKeyCode::Delete)),
            // Ctrl+Tab is inter-pane focus; a bare Tab is the page's own field
            // traversal.
            Key::Tab if !ctrl => send(PaneInput::Key(PaneKeyCode::Tab)),
            _ => {}
        }
    }
}

/// What one frame's copies did, for `--frame-stats`.
/// Drain pane-thread events, maintain seats, and queue accepted texture uploads.
fn drive_pane_host(
    host: Option<ResMut<PaneHost>>,
    bus: Option<Res<PaneBusResource>>,
    failed: Option<Res<PaneHostFailed>>,
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
    // `HudStateChanged` by `cache_hud_state`. Send only changed source revisions;
    // the worker owns per-view success and lifecycle reapplication.
    hud_latest: Res<ViewscreenHudLatest>,
    mut images: ResMut<Assets<Image>>,
    // Where a copied frame is published (issue #1404). The render world empties
    // this at extract and leaves the previous batch's counts behind.
    mut pending: ResMut<PanePendingUploads>,
    mut commands: Commands,
    log: Option<Res<LogFilterConfig>>,
    // `--frame-stats` (see `super::frame_stats`): present only on a host asked
    // to measure, in which case the five phases below are stamped and recorded.
    mut stats: Option<ResMut<PaneFrameStats>>,
    observer: Option<Res<SurfaceObserver>>,
) {
    if failed.is_some() {
        if let Some(bus) = &bus {
            close_after_thread_failure(&bus.0);
        }
        return;
    }
    let Some(mut host) = host else {
        return;
    };
    if host.thread.is_none() {
        // A dead renderer is terminal, including consoles opened by the screen
        // reconciler after the failure. Never spend the per-seat rebuild budget.
        if let Some(bus) = &bus {
            close_after_thread_failure(&bus.0);
        }
        return;
    }
    let started = (stats.is_some() || observer.is_some()).then(Instant::now);
    let mut upload_ns = 0u64;
    let revision = hud_latest.0.revision();
    if host.hud_last_sent_revision != revision {
        host.send(PaneCommand::SetHudScript(
            hud_latest.0.script().map(str::to_owned),
        ));
        host.hud_last_sent_revision = revision;
    }
    if let Some(bus) = &bus {
        retire_closed_panes(&mut host, bus, &mut commands, &log);
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
    let mut sample = PaneFrameSample::default();
    let mut failure = None;
    loop {
        let event = host.thread.as_ref().unwrap().try_recv();
        match event {
            Err(TryRecvError::Empty) => break,
            Err(TryRecvError::Disconnected) => {
                failure = Some("pane thread disconnected".to_owned());
                break;
            }
            Ok(PaneEvent::ThreadFailed { reason }) => {
                failure = Some(reason);
                break;
            }
            Ok(PaneEvent::Created { id, result: Ok(()) }) => {
                if let Some(pane) = host.mirror.get_mut(id) {
                    pane.pending_camera = None;
                }
            }
            Ok(PaneEvent::Created {
                id,
                result: Err(reason),
            }) => {
                let Some(pane) = host.mirror.remove(id) else {
                    continue;
                };
                commands.entity(pane.canvas).try_despawn();
                release_pane_capture(&mut host, id);
                sweep_station_cameras(&mut host, &mut commands);
                host.rebuild_layout();
                crate::pwarn!(
                    log,
                    LogCat::Lobby,
                    "pane host: could not build {id}: {reason}"
                );
                if !pane.kind.permanent() {
                    if let Some(bus) = &bus {
                        bus.0.fault(id, PaneFault::ViewCrashed);
                    }
                }
            }
            Ok(PaneEvent::Loaded(id)) => {
                crate::pinfo!(log, LogCat::Lobby, "pane host: {id} finished loading")
            }
            Ok(PaneEvent::PushDeferred { id, reason }) => crate::pdebug!(
                log,
                LogCat::Lobby,
                "pane host: {id} deferred a push: {reason}"
            ),
            Ok(PaneEvent::Refused { id, refusal }) => {
                crate::pwarn!(log, LogCat::Admit, "pane host: {id}: {refusal}")
            }
            Ok(PaneEvent::CopyFailed {
                id,
                consecutive,
                reason,
            }) => {
                crate::pwarn!(log, LogCat::Lobby,
                    "pane host: {id} frame copy failed ({consecutive}/{VIEW_CRASH_COPY_FAILURES}): {reason}");
                if let Some(fault) = host.mirror.record_copy_failure(id, consecutive) {
                    if let Some(bus) = &bus {
                        bus.0.fault(id, fault);
                    }
                }
            }
            Ok(PaneEvent::Frame(mut frame)) => {
                sample.frames += 1;
                if let Some(trace) = frame.bytes.trace() {
                    trace.drained();
                }
                if !host.mirror.accepts_frame(frame.id, frame.epoch) {
                    sample.stale += 1;
                    if let Some(trace) = frame.bytes.trace_mut() {
                        trace.discarded(if host.mirror.get(frame.id).is_some() {
                            DiscardReason::StaleEpoch
                        } else {
                            DiscardReason::Closed
                        });
                    }
                    continue;
                }
                host.mirror.record_copy_ok(frame.id);
                let pane = host.mirror.get(frame.id).unwrap();
                let upload_start = (stats.is_some() || observer.is_some()).then(Instant::now);
                pending.uploads.push(PaneUpload {
                    image: pane.image.id(),
                    epoch: frame.epoch,
                    rect: frame.rect.into(),
                    surface: pane.raster_size(),
                    full: frame.full,
                    bytes: frame.bytes,
                    attempts: 0,
                });
                upload_ns += elapsed_ns(upload_start);
            }
            Ok(PaneEvent::Stats(iteration)) => {
                if let Some(stats) = stats.as_mut() {
                    stats.record_thread(iteration);
                }
            }
            Ok(PaneEvent::CopyObserved {
                surface,
                observation,
            }) => {
                crate::pinfo!(log, LogCat::Lobby, "{}", observation.log_line(surface));
            }
            Ok(PaneEvent::Started(_)) => {}
        }
    }
    let drain_ns = elapsed_ns(started).saturating_sub(upload_ns);
    if let Some(observer) = &observer {
        observer.record(
            None,
            Operation::MainPass {
                drain_ns,
                queue_ns: upload_ns,
                frames: sample.frames as u64,
            },
        );
    }
    if let Some(reason) = failure {
        if let Some(observer) = &observer {
            observer.record(
                None,
                Operation::Lifecycle {
                    action: "worker_failed",
                },
            );
        }
        crate::perror!(
            log,
            LogCat::Lobby,
            "pane host: {reason}; consoles return to Backfill and the simulation continues"
        );
        if let Some(bus) = &bus {
            close_after_thread_failure(&bus.0);
        }
        let death = host.mirror.thread_death();
        for id in death.fault.into_iter().chain(death.drop_permanent) {
            if let Some(pane) = host.mirror.remove(id) {
                commands.entity(pane.canvas).try_despawn();
            }
            release_pane_capture(&mut host, id);
        }
        sweep_station_cameras(&mut host, &mut commands);
        host.rebuild_layout();
        host.thread.take();
        commands.insert_resource(PaneHostFailed);
        return;
    }
    sample.upload_ms = upload_ns as f64 / 1_000_000.0;
    sample.drain_ms = drain_ns as f64 / 1_000_000.0;
    sample.uploads = pending.tally.uploaded as usize;
    sample.uploads_full = pending.tally.uploaded_full as usize;
    sample.upload_bytes = pending.tally.bytes;
    sample.deferred = pending.tally.deferred as usize;
    sample.lost = pending.tally.dropped as usize;
    if let Some(stats) = stats.as_mut() {
        stats.record_pane(sample);
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
                station_camera: Some(station_camera(host, commands, window).0),
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
        let window = make_pane_view(host, images, commands, new_id, &url, placement);
        host.mirror.insert(new_id, PaneKind::Console, true, window);
        recreated_any = true;
        crate::pinfo!(
            log,
            LogCat::Lobby,
            "pane host: {new_id} ({name}) opening its console on {}",
            if on_station {
                "its Station window"
            } else {
                "the viewscreen window"
            }
        );
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
/// `drive_pane_host` is an ordinary system. A creation or load failure here fails
/// only THIS pane — its station simply stays on Backfill — rather than the whole
/// host, which is right for both a recovery and a console the operator can
/// simply close and re-open.
fn make_pane_view(
    host: &PaneHost,
    images: &mut Assets<Image>,
    commands: &mut Commands,
    id: PaneId,
    url: &str,
    placement: PaneSeat,
) -> PaneCanvasData {
    let PaneSeat {
        window,
        station_camera,
        origin,
        size,
        scale,
        window_origin,
    } = placement;
    let render_scale = host.console_render_scale;
    let raster = render_scale.geometry(size, scale);
    host.send(PaneCommand::Create {
        id,
        kind: PaneKind::Console,
        spec: PaneSpecOwned {
            width: raster.size.0,
            height: raster.size.1,
            device_scale: raster.device_scale,
        },
        url: url.to_owned(),
        epoch: 0,
        visible: true,
    });
    let image = Image::new_fill(
        Extent3d {
            width: raster.size.0,
            height: raster.size.1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        // `transparent: false` above: an opaque console, copied verbatim.
        &pane_fill(false),
        pane_texture_format(false),
        // RENDER_WORLD only — see the note at `init_pane_host`'s twin of this
        // call, and `super::upload`.
        RenderAssetUsages::RENDER_WORLD,
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
    PaneCanvasData {
        image: handle,
        canvas,
        window,
        origin,
        size,
        scale,
        render_scale,
        window_origin,
        pending_camera: station_camera,
    }
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
    if host.mirror.is_empty() {
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
    if host.mirror.iter().all(survives) {
        return;
    }
    let mut closed: Vec<PaneId> = Vec::new();
    host.mirror.retain(|window| {
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

    for pane in &closed {
        host.send(PaneCommand::Close(*pane));
        release_pane_capture(host, *pane);
    }
    // A Station window whose every pane has closed no longer needs its 2-D
    // camera; despawn it so nothing keeps clearing an empty Station to black. The
    // window itself is `bridge_display`'s to own.
    sweep_station_cameras(host, commands);

    // Rebuild the router and reconcile the focus order. `sync_order` keeps the
    // focused pane if it survived and clears focus if it closed — never carrying
    // one participant's focus onto another.
    host.rebuild_layout();
}

fn release_pane_capture(host: &mut PaneHost, id: PaneId) {
    host.contacts.release_pane(id);
    if host.mouse_capture.captured() == Some(id) {
        host.mouse_capture.release();
    }
}

fn sweep_station_cameras(host: &mut PaneHost, commands: &mut Commands) {
    // A pending create may have joined a camera another failed create minted.
    // Live occupancy, not original ownership, decides whether it can be removed.
    host.station_cameras.retain(|(window, camera)| {
        if host.mirror.iter().any(|pane| pane.window == *window) {
            return true;
        }
        commands.entity(*camera).try_despawn();
        false
    });
}

fn report_pane_shutdown_timeout() {
    warn!(target: LogCat::Config.target(),
        "pane host: renderer did not stop within 2 s; leaving its thread detached"
    );
}

fn stop_pane_host_on_exit(
    mut exit: MessageReader<AppExit>,
    host: Option<ResMut<PaneHost>>,
    starting: Option<ResMut<PaneHostStarting>>,
) {
    if exit.read().next().is_none() {
        return;
    }
    if let Some(mut host) = host {
        if let Some(thread) = host.thread.as_mut() {
            thread.stop();
        }
    }
    if let Some(mut starting) = starting {
        starting.0.stop();
    }
}
