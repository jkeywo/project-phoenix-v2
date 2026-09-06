//! The protocol a pane host and its renderer speak, and the seams they speak it
//! through (issue #1404, slice 2).
//!
//! # What this is for
//!
//! Panes are rasterised by one Ultralight renderer, and that renderer has
//! **thread affinity**: its `Renderer` and every `View` must be created, used
//! and dropped on one thread. Today that thread is Bevy's main thread, which is
//! also the simulation's — so ~21 ms of pane work per frame is ~21 ms the ship
//! is not simulating. #1404 moves the renderer onto a thread of its own, and a
//! thread boundary is a place where nothing can be *reached into*: the frame
//! loop can no longer call a `View` method, only send a message and read one
//! back.
//!
//! This module is that message set — [`PaneCommand`] one way, [`PaneEvent`] the
//! other — plus the three traits the loop is written against ([`PaneView`],
//! [`PaneRuntime`], [`PaneFrameSink`]). It is deliberately **Bevy-free and
//! feature-OFF**: the policy that will run on the pane thread (slice 3's
//! `PaneLoop`) is checked by the ordinary `cargo test` every CI job runs,
//! against the doubles below, rather than only by a human on a Windows machine
//! with an SDK and a GPU. That is the same split the rest of
//! [`super`] is built on.
//!
//! In *this* slice nothing is sent anywhere: [`super::ultralight`] implements
//! the traits and calls through them, so the calls, and their order, are exactly
//! what they were.
//!
//! # Latest-wins slots, not queued pushes
//!
//! Two of the commands — [`PaneCommand::SetHudScript`] and
//! [`PaneCommand::SetGamepadScript`] — carry a script the producing side wants
//! *evaluated every iteration while it is held*, not once per send. They are
//! **slots**: sending one replaces whatever the slot held, and the thread
//! re-pushes the current value on its own cadence.
//!
//! The two are pushed at different points of an iteration, and deliberately:
//! the HUD's goes in with the rest of the pumping, *after* `Renderer::update`,
//! so the DOM change it makes is picked up by the `render` below it; the gamepad
//! snapshot goes in *before* `update`, because a console page polls the pads
//! from its own `requestAnimationFrame` callback and that callback is serviced
//! inside `update` — pushing it later would cost a whole iteration of stick
//! latency. See [`PaneLoop::iterate`].
//!
//! That is both what the main thread does today (`cache_hud_state` keeps the
//! newest HUD JSON and `drive_panes` pushes it each frame it draws) and the only
//! shape that is bounded by construction. Queued pushes would let a 60 Hz
//! producer outrun a 45 Hz renderer and grow an unbounded backlog of states
//! nobody will ever see — the newest is the only one that matters, and a slot
//! cannot accumulate.
//!
//! # Why [`PaneKeyCode`] is ten variants
//!
//! Exactly the ten keys `forward_keyboard_text` forwards today — Backspace,
//! Enter, the four arrows, Home, End, Delete and a bare Tab — each as a
//! `RawKeyDown` with native code `0` and default modifiers. Everything else the
//! operator types travels as [`PaneInput::KeyChar`], which is the event that
//! actually puts a character into a field.
//!
//! Enumerating them rather than passing Ultralight's own `VirtualKeyCode`
//! through is what keeps this module SDK-free, and keeping it to ten rather than
//! transcribing the whole table is honesty: a code this side can name but the
//! adapter never sends would be an untested path pretending to be a supported
//! one. Widening it is one variant and one match arm, on the day something sends
//! one.
//!
//! # Order inside a pane's stream is load-bearing
//!
//! [`PaneInput`]s for one pane are a FIFO, and the loop applies them in the
//! order they arrive. That is not incidental tidiness:
//!
//! - Ultralight decides what is under the pointer from the **move**, so a
//!   `MouseDown` that is not preceded by a `MouseMove` to the same point lands
//!   wherever the pointer last was. The press path sends both, in that order.
//! - [`PaneInput::Focus`] must reach a pane **before** the keys aimed at it:
//!   Ultralight drops input into an unfocused view, so a click-then-type in one
//!   frame types into nothing if the two are reordered.
//!
//! Between panes there is no order to keep — two panes are two independent
//! documents — which is why the queue is per pane rather than global.

use std::ops::{Deref, DerefMut};
use std::sync::mpsc::Sender;
use std::time::Instant;

use super::registry::PaneId;
use super::surface::{pump_pane, PaneSurface, PaneSurfaceError};
use super::transport::{PaneBus, PaneInputRefusal};
use crate::native_host::host_lobby::{pump_host_lobby, HostLobbyBridge};

/// What a surface *is*, as far as the pane loop is concerned.
///
/// Three kinds rather than a bag of booleans, because the two questions the loop
/// asks — is it transparent, is it permanent — are answered by the kind and by
/// nothing else, and a surface that answered them independently could be
/// incoherent (a transparent lobby, a permanent console).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaneKind {
    /// A participant's console: an entry on the pane bus, an identity, a station
    /// it can lose to Backfill.
    Console,
    /// The host operator's own lobby chrome (issue #1325). Rides its own bridge,
    /// never the pane bus.
    Lobby,
    /// The viewscreen HUD overlay (issue #422, native port): a passive,
    /// transparent frame over the live 3-D scene.
    Hud,
}

impl PaneKind {
    /// Whether the page composites over what is behind it.
    ///
    /// Only the HUD does. An opaque surface is copied verbatim out of
    /// Ultralight's premultiplied-BGRA buffer; the HUD pays for straight alpha
    /// so the viewscreen shows through wherever it does not paint (issue #1402).
    /// The copy and the texture format are decided by this one predicate on both
    /// sides, so they cannot come apart.
    pub const fn transparent(self) -> bool {
        matches!(self, PaneKind::Hud)
    }

    /// Whether the surface is **permanent**: no pane-bus entry, never retired by
    /// the close sweep, and never faulted.
    ///
    /// The lobby and the HUD both are. Neither holds a station, so neither has a
    /// Backfill to fall back to, and closing the lobby would remove the one
    /// surface the operator drives the host from. A dead permanent view is a
    /// warning and a blank rectangle — honest, and recoverable by restarting the
    /// host — rather than a fault that has nowhere to go.
    pub const fn permanent(self) -> bool {
        matches!(self, PaneKind::Lobby | PaneKind::Hud)
    }
}

/// A view's geometry, owned — the SDK-free half of `vellum_ultralight`'s
/// `PaneSpec`.
///
/// `width`/`height` are **physical** pixels and `device_scale` is the window's
/// scale factor, which is what makes logical cursor coordinates equal page CSS
/// pixels. Owned and Bevy-free so it can cross a channel; the adapter turns it
/// into a real `PaneSpec` (adding the session and the transparency the
/// [`PaneKind`] implies) on the renderer's own thread.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneSpecOwned {
    pub width: u32,
    pub height: u32,
    pub device_scale: f64,
}

/// The rectangle of a surface a frame actually repainted, in pixels, `right`
/// and `bottom` exclusive.
///
/// The same semantics as vellum's `DirtyRect`, but its own type: the protocol
/// should not oblige a future producer to be an Ultralight one, so a test
/// double can build and compare `FrameRect`s without any `vellum_ultralight`
/// type in scope. [`From`] conversions both ways are provided anyway, and they
/// live in *this* module rather than at each call site, because
/// `vellum_ultralight::surface` is the crate's **pure** half — it compiles
/// with the SDK feature off — so depending on it here costs nothing.
/// [`super::upload`] keeps speaking `DirtyRect` — it is talking to the copy
/// that produced it — and converts at the seam.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameRect {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl FrameRect {
    /// The whole of a `width`×`height` surface.
    pub const fn full(width: u32, height: u32) -> Self {
        Self {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        }
    }

    /// Whether the rectangle covers no pixels at all. An empty rectangle is not
    /// an error — it is a page that did not repaint — but nothing may be
    /// uploaded from it.
    pub const fn is_empty(&self) -> bool {
        self.right <= self.left || self.bottom <= self.top
    }

    /// How many pixels the rectangle covers. `u64` because a 4-K surface's count
    /// summed over a run of frames leaves `u32` behind quickly.
    pub const fn pixel_count(&self) -> u64 {
        if self.is_empty() {
            return 0;
        }
        (self.right - self.left) as u64 * (self.bottom - self.top) as u64
    }
}

impl From<vellum_ultralight::surface::DirtyRect> for FrameRect {
    fn from(rect: vellum_ultralight::surface::DirtyRect) -> Self {
        Self {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }
}

impl From<FrameRect> for vellum_ultralight::surface::DirtyRect {
    fn from(rect: FrameRect) -> Self {
        Self {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        }
    }
}

/// The editing keys forwarded to a page as raw key-downs.
///
/// See the module note for why the set is exactly these ten.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaneKeyCode {
    Back,
    Return,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    Delete,
    Tab,
}

/// One thing this machine's operator did to a pane.
///
/// Coordinates are `i32` **page CSS pixels** — the space the router already
/// resolves to, and the space every view call takes today, so nothing is
/// converted at either end of the seam.
#[derive(Clone, Debug, PartialEq)]
pub enum PaneInput {
    MouseMove { x: i32, y: i32 },
    MouseDown { x: i32, y: i32 },
    MouseUp { x: i32, y: i32 },
    Scroll { dx: i32, dy: i32 },
    Key(PaneKeyCode),
    KeyChar(String),
    Focus,
    Unfocus,
}

/// One instruction for the side that owns the views.
#[derive(Clone, Debug, PartialEq)]
pub enum PaneCommand {
    /// Build a view for `id`, and navigate it to `url`.
    Create {
        id: PaneId,
        kind: PaneKind,
        spec: PaneSpecOwned,
        url: String,
        /// The texture generation this view's frames will be current for. A
        /// frame carrying an older epoch describes a surface that no longer
        /// exists.
        epoch: u64,
        /// Whether the surface is drawn. A hidden surface is still pumped — so
        /// a reveal is instant — but is not copied.
        visible: bool,
    },
    /// Drop the view. Permanent surfaces are never closed; see
    /// [`PaneKind::permanent`].
    Close(PaneId),
    /// Resize the view, and start a new texture generation.
    Resize {
        id: PaneId,
        width: u32,
        height: u32,
        epoch: u64,
    },
    SetVisible {
        id: PaneId,
        visible: bool,
    },
    Input {
        id: PaneId,
        input: PaneInput,
    },
    /// The HUD readout to keep pushing, or `None` to stop. A **slot**, not a
    /// queued push — see the module note.
    SetHudScript(Option<String>),
    /// The gamepad snapshot to keep pushing into every console, or `None`.
    /// A slot, like the HUD's.
    SetGamepadScript(Option<String>),
    /// Stop. The last command that will be read.
    Shutdown,
}

/// A frame, and the buffer it was copied into.
///
/// `bytes` is the pane's **full-size** staging buffer (`width * height * 4`,
/// stride `width * 4`) of which only `rect` is current — the shape vellum's copy
/// writes, and the shape `write_texture` takes. Dropping the frame returns the
/// allocation to the producer's pool; see [`PaneFrameBuffer`].
#[derive(Debug)]
pub struct PaneFrame {
    pub id: PaneId,
    pub epoch: u64,
    pub rect: FrameRect,
    /// The rectangle is the **whole** surface because the copy was *forced*, not
    /// merely because the page happened to repaint all of it — the same fact
    /// [`PaneFrameSink::publish`] carries, and for the same two consumers: it
    /// becomes `PaneUpload.full`, which feeds `uploads_full` and lets
    /// `super::upload::defer` promote a deferred upload to whole. Inferring it
    /// from `rect` would be wrong: a page that repaints edge to edge on its own
    /// is not an answer to a force.
    pub full: bool,
    pub bytes: PaneFrameBuffer,
}

/// What the side that owns the views has to say.
#[derive(Debug)]
pub enum PaneEvent {
    /// The renderer started, or could not be started. Always the first event,
    /// and the handshake the seat building waits on.
    Started(Result<(), String>),
    /// A [`PaneCommand::Create`] was carried out, or refused.
    Created {
        id: PaneId,
        result: Result<(), String>,
    },
    /// A pane's document finished loading — the rising edge, once per view.
    Loaded(PaneId),
    /// A pane repainted, and here are the pixels.
    Frame(PaneFrame),
    /// A frame copy failed, and how many in a row have now failed. Past
    /// `VIEW_CRASH_COPY_FAILURES` a non-permanent pane is treated as crashed.
    CopyFailed {
        id: PaneId,
        consecutive: u32,
        reason: String,
    },
    /// A page said something it is not entitled to say, and was refused.
    Refused {
        id: PaneId,
        refusal: PaneInputRefusal,
    },
    /// A push was deferred and its batch put back — ordinarily the window
    /// between "the document loaded" and "its own modules ran".
    PushDeferred { id: PaneId, reason: String },
    /// One iteration's cost, for `--frame-stats`.
    Stats(PaneThreadSample),
    /// The renderer stopped, and no further event will arrive.
    ThreadFailed { reason: String },
}

/// What one iteration of the pane loop cost, and did.
///
/// The pane-side twin of [`super::frame_stats::PaneFrameSample`], which after
/// slice 5 measures only what the *main* thread still spends on panes. Recorded
/// here so an iteration that costs 22 ms is legible as an iteration, not
/// smeared across however many main frames it spanned. Wiring it into the report
/// is slice 5's.
///
/// Its `copied`/`forced`/`pixels` are the **loop's own** counts — what
/// `copy_frame` reported — and can legitimately differ from the sink's tally,
/// which counts only what it actually published and so excludes a frame found
/// stale at publish time (an epoch the pane has moved past). `drive_panes`
/// records the sink's tally for those three, and this sample's phase timings.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaneThreadSample {
    /// `Renderer::update` — the library's own timers, network and script work.
    pub update_ms: f64,
    /// Messages moved both ways across the bridge, for every pane — plus the
    /// pre-update gamepad push, which is pushing too even though it happens
    /// before `update_ms`'s phase rather than after it.
    pub pump_ms: f64,
    /// `Renderer::render` — rasterising whatever repainted.
    pub render_ms: f64,
    /// Copying dirty rectangles out of the surfaces.
    pub copy_ms: f64,
    /// Publishing the copied frames.
    pub publish_ms: f64,
    /// How many views existed this iteration.
    pub panes: usize,
    /// How many produced a frame.
    pub copied: usize,
    /// How many of those were forced whole.
    pub forced: usize,
    /// Pixels copied.
    pub pixels: u64,
    /// The whole iteration, wall clock — always at least the five phases, and
    /// more than them when the loop waited.
    pub iteration_ms: f64,
}

/// One pane's staging buffer, on loan from that pane's pool.
///
/// Dropping it returns the allocation to the producer over an `mpsc::Sender`,
/// which is what makes the buffers a pool rather than an allocation per frame
/// *without* threading a lifetime across the seam: whoever holds a frame last —
/// the render world after its `write_texture`, or a stale frame that was
/// superseded — hands the allocation back by dropping it. A buffer built without
/// a sender (a test fixture, or a producer that does not pool) simply frees.
///
/// It lives in this module, and is re-exported by [`super::upload`], because
/// both halves of the seam carry it and this is the half with no Bevy in it.
#[derive(Debug)]
pub struct PaneFrameBuffer {
    pane: PaneId,
    bytes: Vec<u8>,
    recycle: Option<Sender<(PaneId, Vec<u8>)>>,
}

impl PaneFrameBuffer {
    /// Take `bytes` for `pane`, to be returned through `recycle` on drop.
    pub fn new(pane: PaneId, bytes: Vec<u8>, recycle: Option<Sender<(PaneId, Vec<u8>)>>) -> Self {
        Self {
            pane,
            bytes,
            recycle,
        }
    }

    /// Which pane's pool this buffer belongs to.
    pub fn pane(&self) -> PaneId {
        self.pane
    }
}

impl Deref for PaneFrameBuffer {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.bytes
    }
}

impl DerefMut for PaneFrameBuffer {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.bytes
    }
}

impl Drop for PaneFrameBuffer {
    fn drop(&mut self) {
        if let Some(recycle) = self.recycle.take() {
            let bytes = std::mem::take(&mut self.bytes);
            // A closed receiver means the producer is gone (the pane host was
            // torn down, or the process is exiting); the allocation simply
            // frees here instead.
            let _ = recycle.send((self.pane, bytes));
        }
    }
}

/// One live view, as the pane loop drives it.
///
/// [`PaneSurface`] is what the *message* pump needs (load, ready, push, drain);
/// this adds what the *frame* needs. Split that way round because the pump's
/// policy predates the thread and is already tested through
/// [`RecordingSurface`](super::surface::RecordingSurface) — a view is a surface
/// that can also be sized, pointed at, and copied out of.
pub trait PaneView: PaneSurface {
    /// Ask the document whether it has finished loading, and remember the
    /// answer. Sticky: `is_loading` goes false between navigations too, and a
    /// pane navigates exactly once.
    fn refresh_loaded(&mut self) -> bool;

    /// Resize the view, in physical pixels.
    fn resize(&mut self, width: u32, height: u32);

    /// Deliver one input. Fire-and-forget by construction: nothing is read back,
    /// which is what lets the whole set cross a channel.
    fn input(&mut self, input: &PaneInput);

    /// Copy whatever the page repainted into `dst` — a full-size buffer for this
    /// view's surface — and say which rectangle of it is now current.
    ///
    /// `Ok(None)` is the ordinary still-pane answer and costs nothing. `force`
    /// copies the whole surface regardless of the page's dirty bounds, for the
    /// repaints Ultralight does not flag (a plain attribute write) and for a
    /// texture that is new and holds only its fill.
    fn copy_frame(
        &mut self,
        dst: &mut [u8],
        force: bool,
    ) -> Result<Option<FrameRect>, PaneSurfaceError>;
}

/// The thing that owns the renderer and mints views.
///
/// One per process — Ultralight allows exactly one `Renderer` — and never
/// `Send`: what crosses a thread boundary is the *factory* that builds it, so
/// the affinity is expressed in the type system rather than in a comment.
pub trait PaneRuntime {
    type View: PaneView;

    /// Service the library: timers, network, script.
    fn update(&mut self);

    /// Rasterise whatever repainted.
    fn render(&mut self);

    /// Build a view for `id` and navigate it to `url`.
    ///
    /// A failure here fails **this** pane only: its station stays on Backfill,
    /// or the surface is simply absent, and the host carries on.
    fn create(
        &mut self,
        id: PaneId,
        kind: PaneKind,
        spec: &PaneSpecOwned,
        url: &str,
    ) -> Result<Self::View, PaneSurfaceError>;
}

/// Where copied frames go.
///
/// Two calls rather than one so a frame is copied **into** the pool's own
/// buffer: [`stage`](Self::stage) lends the loop a buffer to copy into, and
/// [`publish`](Self::publish) hands the filled one on. A sink with no buffer
/// free answers `None`, and the loop skips that pane's copy for one iteration
/// rather than allocating a whole surface on the frame path — Ultralight keeps
/// unioning its dirty bounds until the next successful copy, so the pixels are
/// deferred, not lost.
pub trait PaneFrameSink {
    /// Lend a buffer of at least `len` bytes for `id` to copy into, or `None`
    /// when the pane's pool is empty.
    ///
    /// A buffer lent and then **not** published — a still page, a copy that
    /// failed — goes back to its pool; the sink learns that from the next
    /// [`stage`](Self::stage) or from being dropped, not from a third call.
    fn stage(&mut self, id: PaneId, len: usize) -> Option<&mut [u8]>;

    /// Publish the staged buffer as a frame at `epoch` covering `rect`.
    ///
    /// `full` says the rectangle is the **whole** surface because the copy was
    /// forced, not merely because the page happened to repaint all of it. The
    /// consumer needs the distinction rather than being able to infer it: the
    /// render half promotes a partial frame that supersedes a deferred whole one
    /// (`super::upload::defer`) and counts whole uploads separately, and "the
    /// producer asked for the whole surface" is the fact both of those are
    /// about.
    fn publish(&mut self, id: PaneId, epoch: u64, rect: FrameRect, full: bool);

    /// Forget everything held for a pane that has gone.
    fn drop_pane(&mut self, id: PaneId);
}

/// A sink with nothing behind it.
///
/// For the commands that cannot produce a frame — [`PaneCommand::Create`],
/// [`PaneCommand::Input`] and the two script slots — applied from a place where
/// the pool is not reachable. It is safe for [`PaneCommand::Close`] too **on the
/// main-thread wiring of slice 3**, where a pane's staging pool is owned by the
/// Bevy side and dropped with the pane's own record: the real sink's
/// [`drop_pane`](PaneFrameSink::drop_pane) has nothing left to do there. Once
/// the loop owns the pool (slice 5) a `Close` must reach the real sink.
pub struct NoFrameSink;

impl PaneFrameSink for NoFrameSink {
    fn stage(&mut self, _id: PaneId, _len: usize) -> Option<&mut [u8]> {
        None
    }

    fn publish(&mut self, _id: PaneId, _epoch: u64, _rect: FrameRect, _full: bool) {}

    fn drop_pane(&mut self, _id: PaneId) {}
}

/// Whether the loop keeps going.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoopControl {
    Continue,
    Stop,
}

/// One live pane, as the loop drives it.
///
/// Everything here is state the *renderer's* side has to keep between
/// iterations. Its Bevy twin — the texture, the canvas, the rectangle on a
/// monitor — is [`super::mirror::MirrorPane`], and the two are deliberately not
/// the same struct: after slice 5 they are not even on the same thread.
struct LoopPane<V> {
    id: PaneId,
    kind: PaneKind,
    view: V,
    /// The surface's size in physical pixels, which is what a full-size staging
    /// buffer has to be long enough for.
    size: (u32, u32),
    /// The texture generation this view's frames belong to.
    epoch: u64,
    /// Whether the surface is drawn. A hidden surface is still pumped — so a
    /// reveal is instant — but is not copied.
    visible: bool,
    /// Whether the next copy must be forced whole. True at creation — the
    /// texture holds only its fill, so a dirty-rectangle copy would leave the
    /// rest of it blank — and after every resize and reveal, and kept across an
    /// iteration the pane had to skip for want of a buffer.
    needs_full: bool,
    /// Whether something was pushed into this page during **this** iteration's
    /// pump. A push is trusted on its own regardless of what the surface
    /// reports: a plain attribute write is real DOM state that changed and
    /// Ultralight's dirty-bounds tracking does not always flag it.
    pushed_this_iteration: bool,
    /// Consecutive frames whose copy failed. Reset on any success; the count is
    /// reported outward and the *threshold* is the mirror's to apply, because
    /// what a crash means (a station on Backfill, or a blank rectangle) is
    /// decided by what the surface is.
    copy_failures: u32,
}

/// The per-iteration policy: what is driven, in what order, and what comes back.
///
/// This is the whole of what used to be the body of `drive_panes`, lifted out of
/// the Ultralight adapter so that an ordinary `cargo test` can check it. Nothing
/// here knows about Bevy, a GPU or an SDK: the renderer is a [`PaneRuntime`],
/// each document is a [`PaneView`], and the pixels go wherever a
/// [`PaneFrameSink`] puts them.
///
/// The order below is load-bearing and is asserted in the tests:
///
/// 1. `update` — the library's own timers, network and script work;
/// 2. the **pump**, per pane: the load edge, then whichever of the three message
///    routes this kind of surface has;
/// 3. `render` — rasterise whatever the pump made dirty, in one call for every
///    pane;
/// 4. the **copy**, per visible pane: force whole if it was pushed to or owes a
///    whole frame, and publish what came back.
///
/// A push before the render is the point of steps 2 and 3 being in that order: a
/// state pushed after the rasterise would show a frame late, every time.
pub struct PaneLoop<R: PaneRuntime> {
    runtime: R,
    panes: Vec<LoopPane<R::View>>,
    /// The pane bus, while there is one. A host with no `--pane` — the lobby
    /// surface alone — has none at all, and its consoles are simply not pumped.
    bus: Option<PaneBus>,
    /// The host lobby's own bridge. Separate from the bus on purpose: nothing
    /// the lobby says is a participant's `ClientMessage`, so nothing it says may
    /// reach the bus.
    lobby: Option<HostLobbyBridge>,
    /// The HUD readout to keep pushing while it is held — a latest-wins slot,
    /// see the module note.
    hud_script: Option<String>,
    /// The gamepad snapshot to keep pushing into every console, likewise.
    gamepad_script: Option<String>,
    /// Whether to read a clock. Presentation only: an unmeasured host takes no
    /// timestamps at all, and a measured one stamps nothing the simulation can
    /// observe.
    measure: bool,
}

impl<R: PaneRuntime> PaneLoop<R> {
    /// A loop over `runtime`, with no panes and no channels yet.
    pub fn new(runtime: R) -> Self {
        Self {
            runtime,
            panes: Vec::new(),
            bus: None,
            lobby: None,
            hud_script: None,
            gamepad_script: None,
            measure: false,
        }
    }

    /// The pane bus to pump consoles against, or `None` on a host that has no
    /// participants.
    pub fn set_bus(&mut self, bus: Option<PaneBus>) {
        self.bus = bus;
    }

    /// The host lobby's bridge, or `None` on a host with no lobby surface.
    pub fn set_lobby(&mut self, lobby: Option<HostLobbyBridge>) {
        self.lobby = lobby;
    }

    /// Whether to time the phases of each iteration.
    pub fn set_measure(&mut self, measure: bool) {
        self.measure = measure;
    }

    /// Take an already-built view under the loop's management.
    ///
    /// The seat-building path (`init_pane_host`) still constructs the two
    /// permanent surfaces itself, because it distinguishes "the view could not
    /// be created" from "the view could not load its console" in what it logs
    /// and what it does next, and [`PaneRuntime::create`] returns one error for
    /// both. Slice 5 replaces that with a `Create` command and a `Created`
    /// event; until then this is how those views arrive.
    pub fn adopt(&mut self, id: PaneId, kind: PaneKind, view: R::View, size: (u32, u32)) {
        self.panes.push(LoopPane {
            id,
            kind,
            view,
            size,
            epoch: 0,
            visible: true,
            needs_full: true,
            pushed_this_iteration: false,
            copy_failures: 0,
        });
    }

    /// How many views the loop is driving.
    pub fn len(&self) -> usize {
        self.panes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.panes.is_empty()
    }

    /// Whether a pane's view is live here.
    pub fn contains(&self, id: PaneId) -> bool {
        self.panes.iter().any(|p| p.id == id)
    }

    /// One pane's view, for a caller that still delivers input by reaching for
    /// it rather than by sending [`PaneCommand::Input`].
    ///
    /// Slice 4 turns those call sites into commands. Until it does, this is the
    /// same call in the same frame in the same order — which is the point: the
    /// intra-frame order of a `MouseMove` before its `MouseDown`, and of a
    /// `Focus` before the keys aimed at the pane it just gave focus to, is not
    /// something a refactor may quietly reorder.
    pub fn view_mut(&mut self, id: PaneId) -> Option<&mut R::View> {
        self.panes
            .iter_mut()
            .find(|p| p.id == id)
            .map(|p| &mut p.view)
    }

    fn pane_mut(&mut self, id: PaneId) -> Option<&mut LoopPane<R::View>> {
        self.panes.iter_mut().find(|p| p.id == id)
    }

    /// Carry out one instruction, now.
    ///
    /// Everything a command does is synchronous and in-order: a `Create` has
    /// either produced a view or reported why not by the time this returns, and
    /// an `Input` has reached the page. That is what lets the same policy be
    /// driven a command at a time from a Bevy system this slice and from a
    /// channel next.
    pub fn apply(
        &mut self,
        cmd: PaneCommand,
        sink: &mut dyn PaneFrameSink,
        out: &mut Vec<PaneEvent>,
    ) -> LoopControl {
        match cmd {
            PaneCommand::Create {
                id,
                kind,
                spec,
                url,
                epoch,
                visible,
            } => {
                // The load happens inside `create` (slice 2), so there is one
                // answer to report rather than two.
                let result = match self.runtime.create(id, kind, &spec, &url) {
                    Ok(view) => {
                        self.panes.push(LoopPane {
                            id,
                            kind,
                            view,
                            size: (spec.width, spec.height),
                            epoch,
                            visible,
                            // A texture that holds only its fill: the first
                            // frame into it must cover the whole surface.
                            needs_full: true,
                            pushed_this_iteration: false,
                            copy_failures: 0,
                        });
                        Ok(())
                    }
                    Err(e) => Err(e.to_string()),
                };
                out.push(PaneEvent::Created { id, result });
            }
            PaneCommand::Close(id) => {
                // Dropping the view is the teardown: a view left behind is not
                // inert — it would still be pumped, and its page's records would
                // still be drained into a registry that refuses them, once per
                // iteration for the rest of the run.
                self.panes.retain(|p| p.id != id);
                sink.drop_pane(id);
            }
            PaneCommand::Resize {
                id,
                width,
                height,
                epoch,
            } => {
                if let Some(pane) = self.pane_mut(id) {
                    pane.view.resize(width, height);
                    pane.size = (width, height);
                    // The generation is the *other* side's to number — it is the
                    // side that minted the new texture — so it is carried on the
                    // command rather than incremented here.
                    pane.epoch = epoch;
                    pane.needs_full = true;
                }
            }
            PaneCommand::SetVisible { id, visible } => {
                if let Some(pane) = self.pane_mut(id) {
                    // A surface that was hidden has been publishing nothing
                    // while its page carried on repainting, so its texture is
                    // however stale it is: the reveal owes a whole frame.
                    if visible && !pane.visible {
                        pane.needs_full = true;
                    }
                    pane.visible = visible;
                }
            }
            PaneCommand::Input { id, input } => {
                if let Some(pane) = self.pane_mut(id) {
                    pane.view.input(&input);
                }
            }
            PaneCommand::SetHudScript(script) => self.hud_script = script,
            PaneCommand::SetGamepadScript(script) => self.gamepad_script = script,
            PaneCommand::Shutdown => return LoopControl::Stop,
        }
        LoopControl::Continue
    }

    /// One iteration for every pane: service the library, move messages both
    /// ways, rasterise, and copy what repainted.
    pub fn iterate(&mut self, sink: &mut dyn PaneFrameSink, out: &mut Vec<PaneEvent>) {
        let Self {
            runtime,
            panes,
            bus,
            lobby,
            hud_script,
            gamepad_script,
            measure,
        } = self;
        // Presentation time, never simulation time: an unmeasured loop reads no
        // clock here at all — see the module note in `super::frame_stats`.
        let clock = *measure;
        let stamp = |on: bool| on.then(Instant::now);
        let elapsed_ms =
            |from: Option<Instant>| from.map_or(0.0, |t| t.elapsed().as_secs_f64() * 1000.0);
        let iteration = stamp(clock);

        // Pre-update: the gamepad snapshot, and nothing else.
        //
        // A console page reads the pads on its OWN `requestAnimationFrame`
        // callback, and that callback is serviced *inside* `Renderer::update`.
        // So a snapshot pushed after the update is not seen until the next
        // iteration's — one whole iteration of stick latency on every input.
        // The system that fills this slot has always run before the frame that
        // updates the library, and this phase is where that ordering now lives.
        // It is charged to `pump_ms` with the rest of the pushing.
        //
        // Fire-and-forget: a console whose page has not installed the shim yet
        // simply throws, and the next iteration carries the same slot. It does
        // NOT set `pushed_this_iteration` — a snapshot the page polls on its own
        // schedule is not by itself a repaint, so it must not force a whole
        // copy, exactly as the inline `push_gamepads_to_panes` never did.
        let phase = stamp(clock);
        if let Some(script) = &*gamepad_script {
            for pane in panes.iter_mut() {
                if matches!(pane.kind, PaneKind::Console) && pane.view.is_ready() {
                    let _ = pane.view.push(script);
                }
            }
        }
        let mut pump_ms = elapsed_ms(phase);

        let phase = stamp(clock);
        runtime.update();
        let update_ms = elapsed_ms(phase);

        let phase = stamp(clock);
        for pane in panes.iter_mut() {
            pane.pushed_this_iteration = false;
            // A load that has finished is what makes pushes legal. Asking the
            // view each iteration (rather than trusting a callback) keeps this
            // to one place.
            let was_loaded = pane.view.is_ready();
            if pane.view.refresh_loaded() && !was_loaded {
                out.push(PaneEvent::Loaded(pane.id));
            }
            match pane.kind {
                // The HUD overlay is driven by the host's own readout, held in
                // the slot and pushed every iteration it is drawn — one
                // idempotent update on an already-repainting transparent
                // surface, evaluated here so the DOM change is picked up by the
                // render below.
                PaneKind::Hud => {
                    if let (Some(script), true) = (&*hud_script, pane.view.is_ready()) {
                        if pane.view.push(script).is_ok() {
                            pane.pushed_this_iteration = true;
                        }
                    }
                }
                // The lobby surface rides its OWN bridge, over the same
                // `PaneSurface`. Nothing it says is a `ClientMessage` and
                // nothing it hears is a projection, so nothing it says may reach
                // the pane bus — which is the whole reason the two are separate.
                PaneKind::Lobby => {
                    if let Some(bridge) = &*lobby {
                        let report = pump_host_lobby(bridge, &mut pane.view);
                        pane.pushed_this_iteration = report.pushed > 0;
                        if let Some(failure) = &report.push_failure {
                            out.push(PaneEvent::PushDeferred {
                                id: pane.id,
                                reason: failure.to_string(),
                            });
                        }
                    }
                }
                PaneKind::Console => {
                    // The gamepad snapshot went in before `update` above, where
                    // the page's own poll can see it this iteration.
                    let Some(bus) = &*bus else { continue };
                    let report = pump_pane(bus, pane.id, &mut pane.view);
                    pane.pushed_this_iteration = report.pushed > 0;
                    for refusal in report.refusals {
                        out.push(PaneEvent::Refused {
                            id: pane.id,
                            refusal,
                        });
                    }
                }
            }
        }
        pump_ms += elapsed_ms(phase);

        let phase = stamp(clock);
        runtime.render();
        let render_ms = elapsed_ms(phase);

        let phase = stamp(clock);
        let mut copy_ms = 0.0;
        let mut copied = 0usize;
        let mut forced = 0usize;
        let mut pixels = 0u64;
        for pane in panes.iter_mut() {
            // A hidden surface is pumped but not copied: its page stays live, so
            // a reveal is a `display` flip rather than a page load, and the
            // reveal itself owes the whole frame.
            if !pane.visible {
                continue;
            }
            // A push we just made is trusted on its own regardless of what the
            // surface reports. `needs_full` carries a force the pane could not
            // honour — a fresh texture, a resize, or an iteration skipped for
            // want of a buffer.
            let force = pane.pushed_this_iteration || pane.needs_full;
            let len = pane.size.0 as usize * pane.size.1 as usize * 4;
            let staged = match sink.stage(pane.id, len) {
                Some(buffer) => buffer,
                None => {
                    // No buffer free: skip this pane's copy entirely rather than
                    // allocating a whole surface on the frame path. Ultralight
                    // keeps unioning its dirty bounds until the next successful
                    // copy, so nothing is silently lost — but the frame is.
                    pane.needs_full |= force;
                    continue;
                }
            };
            let copy_started = stamp(clock);
            let outcome = pane.view.copy_frame(staged, force);
            copy_ms += elapsed_ms(copy_started);
            match outcome {
                Ok(rect) => {
                    pane.copy_failures = 0;
                    if let Some(rect) = rect {
                        copied += 1;
                        if force {
                            forced += 1;
                        }
                        pixels += rect.pixel_count();
                        // The force has been honoured: the whole surface is in
                        // this buffer, so the texture is whole once it lands.
                        pane.needs_full = false;
                        sink.publish(pane.id, pane.epoch, rect, force);
                    }
                    // `Ok(None)` is a still page. The buffer was never filled,
                    // and the sink takes it back unpublished.
                }
                Err(e) => {
                    // The force was not honoured — a push's repaint may not be
                    // in Ultralight's own dirty bounds — so it is carried to the
                    // next attempt.
                    pane.needs_full |= force;
                    pane.copy_failures += 1;
                    out.push(PaneEvent::CopyFailed {
                        id: pane.id,
                        consecutive: pane.copy_failures,
                        reason: e.to_string(),
                    });
                }
            }
        }
        let publish_ms = (elapsed_ms(phase) - copy_ms).max(0.0);

        out.push(PaneEvent::Stats(PaneThreadSample {
            update_ms,
            pump_ms,
            render_ms,
            copy_ms,
            publish_ms,
            panes: panes.len(),
            copied,
            forced,
            pixels,
            iteration_ms: elapsed_ms(iteration),
        }));
    }
}

#[cfg(test)]
pub(crate) mod doubles {
    //! The test doubles the loop's policy is checked against.
    //!
    //! `pub(crate)` under `cfg(test)` so slice 3's `PaneLoop` tests and slice
    //! 5's thread tests drive the same pair rather than each growing their own
    //! and disagreeing about what a view does.
    //!
    //! That visibility is also a boundary: unlike [`RecordingSurface`], which
    //! is `pub` and reachable from `tests/`, these doubles are only visible to
    //! code inside this crate, so slice 5's pane-thread spawn test has to live
    //! in the lib (a `#[cfg(test)] mod` here or a sibling) rather than as an
    //! integration test.
    //!
    //! `dead_code` is allowed for the same reason: the constructors here are
    //! sized for the loop that will drive them next slice, and trimming them to
    //! whatever this slice's smoke test happens to call would only mean adding
    //! them back.
    #![allow(dead_code)]

    use super::super::surface::RecordingSurface;
    use super::*;

    /// A [`PaneView`] that records instead of rendering.
    #[derive(Debug, Default)]
    pub(crate) struct RecordingView {
        /// The message-pump half, unchanged — so what `pump_pane` decides is
        /// still checked by the double that already checks it.
        pub surface: RecordingSurface,
        /// How this view names itself in [`trace`](Self::trace). Set by
        /// [`RecordingRuntime::create`] to the pane's id.
        pub label: String,
        /// The shared order log, when the runtime that minted this view was
        /// given one.
        ///
        /// The loop's *order* — update, then the pump, then render, then the
        /// copies — is a claim about calls made on two different objects, so no
        /// log kept by either one alone can check it. One log both write into
        /// can, and this is the only reason the doubles share anything.
        pub trace: Option<std::rc::Rc<std::cell::RefCell<Vec<String>>>>,
        /// Every input delivered, in order. The order is the assertion: a
        /// `MouseDown` unpreceded by its `MouseMove`, or keys ahead of their
        /// `Focus`, are the two bugs the seam can introduce.
        pub inputs: Vec<PaneInput>,
        /// Every size this view was asked to take.
        pub resizes: Vec<(u32, u32)>,
        /// What the next copy reports as repainted. `None` is a still page.
        pub paint: Option<FrameRect>,
        /// How many copies to fail before answering normally again — the
        /// crashed-view signal, counted by the loop.
        pub fail_copies: u32,
        /// How many times [`PaneView::copy_frame`] was asked to force.
        pub forced_copies: usize,
        /// Sticky "has finished loading", mirroring
        /// `UltralightPaneSurface::loaded`. Set by [`Self::refresh_loaded`]
        /// once `surface.is_ready()` and [`Self::finishes_loading_after`] calls
        /// have both been satisfied; never cleared.
        pub loaded: bool,
        /// How many [`Self::refresh_loaded`] calls to answer `false` to, once
        /// `surface.is_ready()`, before the rising edge fires. `0` (the
        /// default) flips on the first such call — the common case, where a
        /// double's document is ready the instant the surface is.
        pub finishes_loading_after: usize,
        /// Calls counted toward `finishes_loading_after`, once the surface has
        /// been ready. Not reset — a pane navigates exactly once, so `loaded`
        /// is sticky and so is this.
        pub(crate) calls_while_ready: usize,
    }

    impl RecordingView {
        fn trace(&self, what: &str) {
            if let Some(trace) = &self.trace {
                trace.borrow_mut().push(format!("{what}:{}", self.label));
            }
        }

        /// A view whose document is already loaded.
        pub(crate) fn ready() -> Self {
            Self {
                surface: RecordingSurface::ready(),
                ..Default::default()
            }
        }

        /// A view that will report `rect` repainted on every copy.
        pub(crate) fn painting(rect: FrameRect) -> Self {
            Self {
                paint: Some(rect),
                ..Self::ready()
            }
        }
    }

    impl PaneSurface for RecordingView {
        fn load(&mut self, url: &str) -> Result<(), PaneSurfaceError> {
            self.surface.load(url)
        }

        fn is_ready(&self) -> bool {
            self.loaded
        }

        fn push(&mut self, script: &str) -> Result<(), PaneSurfaceError> {
            self.trace("push");
            self.surface.push(script)
        }

        fn drain(&mut self) -> Vec<String> {
            self.surface.drain()
        }
    }

    impl PaneView for RecordingView {
        fn refresh_loaded(&mut self) -> bool {
            if !self.loaded && self.surface.is_ready() {
                if self.calls_while_ready >= self.finishes_loading_after {
                    self.loaded = true;
                } else {
                    self.calls_while_ready += 1;
                }
            }
            self.loaded
        }

        fn resize(&mut self, width: u32, height: u32) {
            self.resizes.push((width, height));
        }

        fn input(&mut self, input: &PaneInput) {
            self.inputs.push(input.clone());
        }

        fn copy_frame(
            &mut self,
            dst: &mut [u8],
            force: bool,
        ) -> Result<Option<FrameRect>, PaneSurfaceError> {
            self.trace("copy");
            if force {
                self.forced_copies += 1;
            }
            if self.fail_copies > 0 {
                self.fail_copies -= 1;
                return Err(PaneSurfaceError::Frame("no surface".to_string()));
            }
            let rect = if force {
                self.paint
            } else {
                self.paint.filter(|r| !r.is_empty())
            };
            if rect.is_some() {
                // Write something recognisable into the buffer so a test can
                // tell a published buffer from an untouched pooled one.
                if let Some(byte) = dst.first_mut() {
                    *byte = 0xAB;
                }
            }
            Ok(rect)
        }
    }

    /// One frame a [`RecordingSink`] was handed.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) struct PublishedFrame {
        pub id: PaneId,
        pub epoch: u64,
        pub rect: FrameRect,
        /// Whether the producer forced the whole surface.
        pub full: bool,
        /// The first byte of the buffer as it was published —
        /// [`RecordingView::copy_frame`] stamps `0xAB` there, so a test can tell
        /// a filled buffer from an untouched pooled one.
        pub first_byte: u8,
    }

    /// A [`PaneFrameSink`] with a pool per pane and a record of what was
    /// published.
    ///
    /// The pool is the point rather than a detail: "no buffer free" is a real
    /// state of the shipping sink (the render world is a frame behind and has
    /// not handed one back yet), and what the loop does about it — skip the
    /// copy, keep the force — is policy worth a test.
    #[derive(Debug, Default)]
    pub(crate) struct RecordingSink {
        pools: std::collections::HashMap<PaneId, Vec<Vec<u8>>>,
        /// The buffer currently on loan, and whose it is. Returned to its pool
        /// unpublished when the next `stage` comes or the sink is dropped —
        /// which is exactly what the real sink does with a still page's buffer.
        staged: Option<(PaneId, Vec<u8>)>,
        /// Every frame handed over, in order.
        pub published: Vec<PublishedFrame>,
        /// Panes whose pool was dropped.
        pub dropped: Vec<PaneId>,
        /// Lend nothing at all, whatever the pool holds.
        pub starve: bool,
        /// How many `stage` calls found nothing to lend.
        pub starved: usize,
        /// How many buffers a pane's pool is minted with.
        pub buffers_per_pane: usize,
    }

    impl RecordingSink {
        /// A sink whose panes each get two buffers — the steady-state depth of
        /// the shipping pool.
        pub(crate) fn new() -> Self {
            Self {
                buffers_per_pane: 2,
                ..Default::default()
            }
        }

        /// Frames published for one pane.
        pub(crate) fn frames_for(&self, id: PaneId) -> Vec<&PublishedFrame> {
            self.published.iter().filter(|f| f.id == id).collect()
        }

        fn return_staged(&mut self) {
            if let Some((id, bytes)) = self.staged.take() {
                self.pools.entry(id).or_default().push(bytes);
            }
        }
    }

    impl PaneFrameSink for RecordingSink {
        fn stage(&mut self, id: PaneId, len: usize) -> Option<&mut [u8]> {
            self.return_staged();
            if self.starve {
                self.starved += 1;
                return None;
            }
            let buffers = self.buffers_per_pane;
            let pool = self
                .pools
                .entry(id)
                .or_insert_with(|| (0..buffers).map(|_| vec![0u8; len]).collect());
            let Some(mut bytes) = pool.pop() else {
                self.starved += 1;
                return None;
            };
            // A resize changes the surface's length; the shipping pool is
            // re-minted at the new one, and this is the double's equivalent.
            bytes.resize(len, 0);
            self.staged = Some((id, bytes));
            self.staged.as_mut().map(|(_, bytes)| &mut bytes[..])
        }

        fn publish(&mut self, id: PaneId, epoch: u64, rect: FrameRect, full: bool) {
            let Some((staged_id, bytes)) = self.staged.take() else {
                panic!("published a frame for {id} with nothing staged");
            };
            assert_eq!(
                staged_id, id,
                "published a frame into another pane's buffer"
            );
            self.published.push(PublishedFrame {
                id,
                epoch,
                rect,
                full,
                first_byte: bytes.first().copied().unwrap_or(0),
            });
            // The shipping sink's buffer comes back from the render world after
            // its upload; here it goes straight back to the pool.
            self.pools.entry(id).or_default().push(bytes);
        }

        fn drop_pane(&mut self, id: PaneId) {
            self.return_staged();
            self.pools.remove(&id);
            self.dropped.push(id);
        }
    }

    /// Which phase of an iteration ran, and in what order.
    #[derive(Clone, Debug, PartialEq, Eq)]
    pub(crate) enum RuntimePhase {
        Update,
        Render,
        Create(PaneId),
    }

    /// A [`PaneRuntime`] that logs its phases and hands out [`RecordingView`]s.
    #[derive(Debug, Default)]
    pub(crate) struct RecordingRuntime {
        /// Every phase, in order — the loop's shape, asserted rather than
        /// assumed: update, then the pump, then render, then the copies.
        pub phases: Vec<RuntimePhase>,
        /// Panes whose creation must fail, keyed by which pane, with the
        /// reason. Per-pane rather than a single global switch: one seat's
        /// renderer failure (a bad URL, an exhausted view budget) should not
        /// fail every other seat's `create` alongside it.
        pub fail_create: std::collections::HashMap<PaneId, String>,
        /// The shared order log, handed to every view this runtime mints — see
        /// [`RecordingView::trace`].
        pub trace: Option<std::rc::Rc<std::cell::RefCell<Vec<String>>>>,
    }

    impl RecordingRuntime {
        /// A runtime that logs its phases, and its views' pushes and copies,
        /// into one shared list.
        pub(crate) fn traced() -> (Self, std::rc::Rc<std::cell::RefCell<Vec<String>>>) {
            let trace = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
            (
                Self {
                    trace: Some(trace.clone()),
                    ..Default::default()
                },
                trace,
            )
        }

        fn trace(&self, what: &str) {
            if let Some(trace) = &self.trace {
                trace.borrow_mut().push(what.to_string());
            }
        }
    }

    impl PaneRuntime for RecordingRuntime {
        type View = RecordingView;

        fn update(&mut self) {
            self.trace("update");
            self.phases.push(RuntimePhase::Update);
        }

        fn render(&mut self) {
            self.trace("render");
            self.phases.push(RuntimePhase::Render);
        }

        fn create(
            &mut self,
            id: PaneId,
            _kind: PaneKind,
            _spec: &PaneSpecOwned,
            url: &str,
        ) -> Result<Self::View, PaneSurfaceError> {
            self.phases.push(RuntimePhase::Create(id));
            if let Some(reason) = self.fail_create.get(&id) {
                return Err(PaneSurfaceError::Load(reason.clone()));
            }
            let mut view = RecordingView {
                label: id.to_string(),
                trace: self.trace.clone(),
                ..RecordingView::ready()
            };
            view.load(url)?;
            Ok(view)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::doubles::{RecordingRuntime, RecordingView, RuntimePhase};
    use super::*;
    use vellum_ultralight::surface::DirtyRect;

    const PANE: PaneId = PaneId(1);

    #[test]
    fn only_the_hud_is_transparent_and_only_it_and_the_lobby_are_permanent() {
        // The copy and the texture format follow `transparent`, and the fault
        // path follows `permanent`: a console that reported itself permanent
        // would never flip its station to Backfill when its view died.
        assert!(PaneKind::Hud.transparent());
        assert!(!PaneKind::Console.transparent());
        assert!(!PaneKind::Lobby.transparent());

        assert!(PaneKind::Hud.permanent());
        assert!(PaneKind::Lobby.permanent());
        assert!(!PaneKind::Console.permanent());
    }

    #[test]
    fn a_full_rect_covers_the_surface_and_an_inverted_one_covers_nothing() {
        let full = FrameRect::full(1920, 1080);
        assert!(!full.is_empty());
        assert_eq!(full.pixel_count(), 1920 * 1080);

        assert!(FrameRect::default().is_empty());
        assert_eq!(FrameRect::default().pixel_count(), 0);
        let inverted = FrameRect {
            left: 10,
            top: 10,
            right: 4,
            bottom: 4,
        };
        assert!(inverted.is_empty());
        assert_eq!(inverted.pixel_count(), 0, "an empty rect counts no pixels");
    }

    #[test]
    fn a_rect_survives_the_round_trip_through_vellums_own() {
        // The render half (`super::upload`) speaks `DirtyRect` because it is
        // talking to the copy that produced one; the protocol speaks
        // `FrameRect`. The conversion is where they meet, so it is the one
        // place a transposed edge would be silent.
        let rect = FrameRect {
            left: 3,
            top: 7,
            right: 40,
            bottom: 90,
        };
        let dirty: DirtyRect = rect.into();
        assert_eq!(dirty.left, 3);
        assert_eq!(dirty.top, 7);
        assert_eq!(dirty.right, 40);
        assert_eq!(dirty.bottom, 90);
        assert_eq!(FrameRect::from(dirty), rect);
        assert_eq!(dirty.pixel_count(), rect.pixel_count());
        assert_eq!(
            FrameRect::from(DirtyRect::full(64, 32)),
            FrameRect::full(64, 32)
        );
    }

    #[test]
    fn a_dropped_frame_buffer_goes_back_to_its_own_pane() {
        let (tx, rx) = std::sync::mpsc::channel();
        drop(PaneFrameBuffer::new(PANE, vec![7; 12], Some(tx)));
        let (pane, bytes) = rx.try_recv().expect("the allocation came back");
        assert_eq!(pane, PANE);
        assert_eq!(bytes.len(), 12);
    }

    #[test]
    fn the_doubles_satisfy_the_seams_the_loop_is_written_against() {
        // A smoke test with a purpose: slice 3's loop is generic over these
        // traits, so a double that no longer satisfies one of them is a
        // compile error here rather than in the policy tests that matter.
        fn drive<R: PaneRuntime>(runtime: &mut R, id: PaneId, url: &str) -> R::View {
            runtime.update();
            let view = runtime
                .create(
                    id,
                    PaneKind::Console,
                    &PaneSpecOwned {
                        width: 8,
                        height: 4,
                        device_scale: 1.0,
                    },
                    url,
                )
                .expect("the double creates");
            runtime.render();
            view
        }

        let mut runtime = RecordingRuntime::default();
        let mut view = drive(&mut runtime, PANE, "http://127.0.0.1/pane");
        assert_eq!(
            runtime.phases,
            vec![
                RuntimePhase::Update,
                RuntimePhase::Create(PANE),
                RuntimePhase::Render
            ]
        );
        assert_eq!(view.surface.loaded, vec!["http://127.0.0.1/pane"]);
        assert!(view.refresh_loaded());

        view.resize(16, 8);
        view.input(&PaneInput::MouseMove { x: 2, y: 3 });
        view.input(&PaneInput::MouseDown { x: 2, y: 3 });
        view.input(&PaneInput::Key(PaneKeyCode::Return));
        assert_eq!(view.resizes, vec![(16, 8)]);
        assert_eq!(
            view.inputs,
            vec![
                PaneInput::MouseMove { x: 2, y: 3 },
                PaneInput::MouseDown { x: 2, y: 3 },
                PaneInput::Key(PaneKeyCode::Return),
            ],
            "the order a pane's inputs arrive in is the order it sees them"
        );

        let mut dst = vec![0u8; 8 * 4 * 4];
        assert_eq!(view.copy_frame(&mut dst, false), Ok(None), "a still page");
        view.paint = Some(FrameRect::full(8, 4));
        assert_eq!(
            view.copy_frame(&mut dst, true),
            Ok(Some(FrameRect::full(8, 4)))
        );
        view.fail_copies = 1;
        assert!(view.copy_frame(&mut dst, false).is_err());
        assert!(view.copy_frame(&mut dst, false).is_ok(), "one failure only");
    }

    #[test]
    fn a_runtime_that_cannot_create_says_so_rather_than_handing_back_a_view() {
        const OTHER: PaneId = PaneId(2);
        let mut runtime = RecordingRuntime {
            fail_create: std::collections::HashMap::from([(PANE, "no renderer".to_string())]),
            ..Default::default()
        };
        let result = runtime.create(
            PANE,
            PaneKind::Lobby,
            &PaneSpecOwned {
                width: 4,
                height: 4,
                device_scale: 2.0,
            },
            "http://127.0.0.1/lobby",
        );
        assert!(matches!(result, Err(PaneSurfaceError::Load(_))));

        // Failure is per-pane: a seat that was not told to fail still creates
        // fine, even though another seat's create call just failed.
        let other = runtime.create(
            OTHER,
            PaneKind::Console,
            &PaneSpecOwned {
                width: 4,
                height: 4,
                device_scale: 1.0,
            },
            "http://127.0.0.1/console",
        );
        assert!(other.is_ok());

        assert_eq!(
            runtime.phases,
            vec![RuntimePhase::Create(PANE), RuntimePhase::Create(OTHER)]
        );
    }

    #[test]
    fn refresh_loaded_reports_the_rising_edge_exactly_once() {
        // Mirrors `UltralightPaneSurface::refresh_loaded`: `is_ready` (its
        // sticky `loaded`) only becomes true through `refresh_loaded`, and the
        // loop's "finished loading" log line fires on `refresh_loaded() &&
        // !was_loaded` — so the double must actually produce a false-then-true
        // transition, not just echo `is_ready` back at itself.
        let mut view = RecordingView {
            finishes_loading_after: 2,
            ..RecordingView::ready()
        };
        assert!(!view.is_ready(), "not loaded until refresh says so");

        // Two calls before the document reports finished loading.
        assert!(!view.refresh_loaded());
        assert!(!view.is_ready());
        assert!(!view.refresh_loaded());
        assert!(!view.is_ready());

        // Third call: the rising edge, observed the way the loop observes it.
        let was_loaded = view.is_ready();
        assert!(
            view.refresh_loaded() && !was_loaded,
            "the edge fires exactly once, on this call"
        );
        assert!(view.is_ready());

        // Every call after stays true — there is no second edge.
        for _ in 0..3 {
            let was_loaded = view.is_ready();
            assert!(view.refresh_loaded());
            assert!(was_loaded, "already loaded, so this is not an edge");
        }
    }
}

#[cfg(test)]
mod loop_tests {
    //! What one iteration of [`PaneLoop`] does, and in what order (issue #1404,
    //! slice 3).
    //!
    //! Every claim here used to be a claim about `drive_panes`, provable only by
    //! a human on a Windows machine with an SDK and a GPU watching four
    //! consoles. They are the claims that decide whether a console draws at all:
    //! that a push reaches a page before the rasterise that would show it, that
    //! a page pushed to is copied WHOLE (Ultralight does not flag every
    //! repaint), that a run of failed copies is counted rather than smoothed
    //! over, and that a closed pane stops being driven.

    use super::doubles::{RecordingRuntime, RecordingSink};
    use super::*;
    use crate::core::messages::{DeliveryClass, ServerMessage};
    use crate::lobby::handler::Target;
    use crate::native_host::panes::identity::PaneIdentity;
    use crate::native_host::transport::{NativeTransport, TransportDispatch};

    const CONSOLE: PaneId = PaneId(1);
    const LOBBY: PaneId = PaneId(900);
    const HUD: PaneId = PaneId(901);
    const WIDTH: u32 = 4;
    const HEIGHT: u32 = 2;

    fn create(id: PaneId, kind: PaneKind) -> PaneCommand {
        PaneCommand::Create {
            id,
            kind,
            spec: PaneSpecOwned {
                width: WIDTH,
                height: HEIGHT,
                device_scale: 1.0,
            },
            url: "http://127.0.0.1/pane".to_string(),
            epoch: 0,
            visible: true,
        }
    }

    /// A loop driving one pane of `kind`, whose page repaints its whole surface
    /// whenever it is copied.
    fn one_pane(id: PaneId, kind: PaneKind) -> PaneLoop<RecordingRuntime> {
        let mut driver = PaneLoop::new(RecordingRuntime::default());
        let mut out = Vec::new();
        assert_eq!(
            driver.apply(create(id, kind), &mut NoFrameSink, &mut out),
            LoopControl::Continue
        );
        assert!(matches!(
            out.as_slice(),
            [PaneEvent::Created { result: Ok(()), .. }]
        ));
        driver.view_mut(id).expect("it was created").paint = Some(FrameRect::full(WIDTH, HEIGHT));
        driver
    }

    fn bus_with_pane() -> (PaneBus, PaneId) {
        let bus = PaneBus::default();
        let id =
            bus.open(PaneIdentity::adopt("3f1a6c2e-0a11-4b3c-9d55-000000000001", "Ada").unwrap());
        (bus, id)
    }

    fn broadcast(bus: &PaneBus, msg: ServerMessage) {
        bus.transport().dispatch(TransportDispatch {
            target: &Target::All,
            msg: &msg,
            delivery: DeliveryClass::Reliable,
        });
    }

    fn stats(out: &[PaneEvent]) -> PaneThreadSample {
        match out.last() {
            Some(PaneEvent::Stats(sample)) => *sample,
            other => panic!("every iteration ends with its own cost, not {other:?}"),
        }
    }

    #[test]
    fn an_iteration_pushes_pads_then_updates_then_pumps_then_renders_then_copies() {
        // The order is the whole design. A push made after the rasterise shows a
        // frame late, every time; a copy made before it copies the frame before
        // the one the pump just caused.
        //
        // The gamepad snapshot is the one push that goes in AHEAD of `update`:
        // a console polls the pads from its own `requestAnimationFrame`, which
        // `update` is what services, so a snapshot pushed after it would be read
        // an iteration late — a whole iteration of stick latency.
        let (runtime, trace) = RecordingRuntime::traced();
        let mut driver = PaneLoop::new(runtime);
        let mut out = Vec::new();
        driver.apply(create(HUD, PaneKind::Hud), &mut NoFrameSink, &mut out);
        driver.view_mut(HUD).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));
        driver.apply(
            create(CONSOLE, PaneKind::Console),
            &mut NoFrameSink,
            &mut out,
        );
        driver.view_mut(CONSOLE).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));
        driver.apply(
            PaneCommand::SetHudScript(Some("window.__updateHud({})".to_string())),
            &mut NoFrameSink,
            &mut out,
        );
        driver.apply(
            PaneCommand::SetGamepadScript(Some("window.__phoenixSetGamepads([])".to_string())),
            &mut NoFrameSink,
            &mut out,
        );

        let mut sink = RecordingSink::new();
        // One iteration to settle the load edges: a push, of either kind, goes
        // only into a document that has finished loading.
        driver.iterate(&mut sink, &mut out);
        sink.published.clear();
        out.clear();
        trace.borrow_mut().clear();
        driver.iterate(&mut sink, &mut out);

        assert_eq!(
            *trace.borrow(),
            vec![
                format!("push:{CONSOLE}"),
                "update".to_string(),
                format!("push:{HUD}"),
                "render".to_string(),
                format!("copy:{HUD}"),
                format!("copy:{CONSOLE}"),
            ]
        );
        let pad_push = trace
            .borrow()
            .iter()
            .position(|t| t == &format!("push:{CONSOLE}"))
            .expect("the pad snapshot is pushed");
        let update = trace
            .borrow()
            .iter()
            .position(|t| t == "update")
            .expect("the library is updated");
        assert!(
            pad_push < update,
            "the pad snapshot must reach the page before the update that lets it read them"
        );
        // The first iteration cleared both panes' opening `needs_full`, so what
        // forces a copy here is only this iteration's pushing. The HUD's push
        // does; the console's pad snapshot does NOT — a snapshot the page polls
        // on its own schedule is not by itself a repaint, and the inline
        // `push_gamepads_to_panes` never counted one either.
        let hud = sink
            .published
            .iter()
            .find(|f| f.id == HUD)
            .expect("the HUD publishes");
        assert!(hud.full, "a HUD push forces a whole copy");
        let console = sink
            .published
            .iter()
            .find(|f| f.id == CONSOLE)
            .expect("the console publishes");
        assert!(!console.full, "a gamepad push does not force a copy");
    }

    #[test]
    fn a_document_that_finishes_loading_says_so_exactly_once() {
        let mut driver = PaneLoop::new(RecordingRuntime::default());
        let mut out = Vec::new();
        driver.apply(
            create(CONSOLE, PaneKind::Console),
            &mut NoFrameSink,
            &mut out,
        );
        // Two iterations of "still loading" before the edge.
        driver.view_mut(CONSOLE).unwrap().finishes_loading_after = 2;
        let mut sink = RecordingSink::new();

        let mut edges = 0;
        for _ in 0..5 {
            out.clear();
            driver.iterate(&mut sink, &mut out);
            edges += out
                .iter()
                .filter(|e| matches!(e, PaneEvent::Loaded(id) if *id == CONSOLE))
                .count();
        }
        assert_eq!(edges, 1, "the rising edge, not the state");
    }

    #[test]
    fn a_page_that_was_pushed_to_is_copied_whole_and_a_quiet_one_is_not() {
        // Ultralight's dirty-bounds tracking does not flag every repaint — a
        // plain attribute write is real DOM state that changed and is not in
        // them — so a push is trusted on its own. Without this a console that
        // updates one readout shows the update only when something else forces
        // a whole copy.
        let mut driver = one_pane(HUD, PaneKind::Hud);
        let mut sink = RecordingSink::new();
        let mut out = Vec::new();
        driver.apply(
            PaneCommand::SetHudScript(Some("window.__updateHud({})".to_string())),
            &mut NoFrameSink,
            &mut out,
        );

        driver.iterate(&mut sink, &mut out);
        driver.iterate(&mut sink, &mut out);
        // The slot is dropped: nothing is pushed, and nothing is forced.
        driver.apply(PaneCommand::SetHudScript(None), &mut NoFrameSink, &mut out);
        out.clear();
        driver.iterate(&mut sink, &mut out);

        let frames = sink.frames_for(HUD);
        assert_eq!(
            frames.iter().map(|f| f.full).collect::<Vec<_>>(),
            vec![true, true, false],
            "the first owes a whole texture, the second was pushed to, the third is quiet"
        );
        assert!(
            frames.iter().all(|f| f.first_byte == 0xAB),
            "every published buffer carries what the copy wrote"
        );
        assert_eq!(stats(&out).copied, 1);
        assert_eq!(stats(&out).forced, 0);
    }

    #[test]
    fn failed_copies_count_up_in_a_run_and_one_success_ends_it() {
        // A single failure is a transient — a repaint mid-flight, a buffer not
        // ready. A view that has genuinely died fails every frame, so the run is
        // the signal, and the count is what the mirror's threshold reads.
        let mut driver = one_pane(CONSOLE, PaneKind::Console);
        driver.view_mut(CONSOLE).unwrap().fail_copies = 3;
        let mut sink = RecordingSink::new();
        let mut out = Vec::new();

        let mut runs = Vec::new();
        for _ in 0..4 {
            out.clear();
            driver.iterate(&mut sink, &mut out);
            runs.extend(out.iter().filter_map(|e| match e {
                PaneEvent::CopyFailed { consecutive, .. } => Some(*consecutive),
                _ => None,
            }));
        }
        assert_eq!(runs, vec![1, 2, 3], "consecutive, then a success");
        assert_eq!(sink.published.len(), 1, "the fourth iteration published");
        assert!(
            sink.frames_for(CONSOLE)[0].full,
            "the force the failed copies could not honour was carried, not dropped"
        );

        // And the next failure starts a fresh run rather than resuming the old.
        driver.view_mut(CONSOLE).unwrap().fail_copies = 1;
        out.clear();
        driver.iterate(&mut sink, &mut out);
        assert!(out
            .iter()
            .any(|e| matches!(e, PaneEvent::CopyFailed { consecutive: 1, .. })));
    }

    #[test]
    fn a_closed_pane_is_dropped_and_stops_being_driven() {
        // A view left behind is not inert: it would still be pumped, and its
        // page's records would still be drained into a registry that refuses
        // them, once per iteration for the rest of the run.
        let mut driver = one_pane(CONSOLE, PaneKind::Console);
        let mut sink = RecordingSink::new();
        let mut out = Vec::new();
        driver.iterate(&mut sink, &mut out);
        assert_eq!(sink.published.len(), 1);

        driver.apply(PaneCommand::Close(CONSOLE), &mut sink, &mut out);
        assert!(!driver.contains(CONSOLE));
        assert_eq!(sink.dropped, vec![CONSOLE], "and its pool went with it");

        out.clear();
        driver.iterate(&mut sink, &mut out);
        assert_eq!(sink.published.len(), 1, "nothing more was copied");
        assert_eq!(stats(&out).panes, 0);
    }

    #[test]
    fn a_view_that_cannot_be_created_is_reported_and_leaves_no_pane_behind() {
        // A per-seat failure fails THIS pane only — its station simply stays on
        // Backfill — rather than the whole host.
        let mut driver = PaneLoop::new(RecordingRuntime {
            fail_create: std::collections::HashMap::from([(CONSOLE, "no renderer".to_string())]),
            ..Default::default()
        });
        let mut out = Vec::new();
        driver.apply(
            create(CONSOLE, PaneKind::Console),
            &mut NoFrameSink,
            &mut out,
        );
        match out.as_slice() {
            [PaneEvent::Created {
                id,
                result: Err(reason),
            }] => {
                assert_eq!(*id, CONSOLE);
                assert_eq!(reason, "load failed: no renderer");
            }
            other => panic!("expected one refusal, got {other:?}"),
        }
        assert!(driver.is_empty());

        // The other seat still builds, and the loop drives it.
        out.clear();
        driver.apply(create(LOBBY, PaneKind::Lobby), &mut NoFrameSink, &mut out);
        assert!(matches!(
            out.as_slice(),
            [PaneEvent::Created { result: Ok(()), .. }]
        ));
        assert_eq!(driver.len(), 1);
    }

    #[test]
    fn a_panes_inputs_reach_its_view_in_the_order_they_were_sent() {
        // Ultralight decides what is under the pointer from the MOVE, and drops
        // input into an unfocused view — so a `MouseDown` ahead of its
        // `MouseMove` lands wherever the pointer last was, and keys ahead of
        // their `Focus` type into nothing.
        let mut driver = one_pane(CONSOLE, PaneKind::Console);
        let mut out = Vec::new();
        let sent = [
            PaneInput::Focus,
            PaneInput::MouseMove { x: 3, y: 4 },
            PaneInput::MouseDown { x: 3, y: 4 },
            PaneInput::KeyChar("a".to_string()),
            PaneInput::Key(PaneKeyCode::Return),
            PaneInput::MouseUp { x: 3, y: 4 },
            PaneInput::Unfocus,
        ];
        for input in sent.iter().cloned() {
            driver.apply(
                PaneCommand::Input { id: CONSOLE, input },
                &mut NoFrameSink,
                &mut out,
            );
        }
        assert_eq!(driver.view_mut(CONSOLE).unwrap().inputs, sent.to_vec());
        assert!(out.is_empty(), "input is fire-and-forget");

        // An input for a pane that has gone is dropped, not a panic.
        driver.apply(PaneCommand::Close(CONSOLE), &mut NoFrameSink, &mut out);
        driver.apply(
            PaneCommand::Input {
                id: CONSOLE,
                input: PaneInput::Focus,
            },
            &mut NoFrameSink,
            &mut out,
        );
    }

    #[test]
    fn commands_have_landed_before_the_iteration_that_follows_them() {
        let mut driver = one_pane(CONSOLE, PaneKind::Console);
        let mut sink = RecordingSink::new();
        let mut out = Vec::new();
        driver.iterate(&mut sink, &mut out);

        driver.apply(
            PaneCommand::Input {
                id: CONSOLE,
                input: PaneInput::MouseMove { x: 1, y: 1 },
            },
            &mut NoFrameSink,
            &mut out,
        );
        out.clear();
        driver.iterate(&mut sink, &mut out);
        assert_eq!(
            driver.view_mut(CONSOLE).unwrap().inputs.len(),
            1,
            "delivered once, before this iteration rather than during it"
        );
        assert_eq!(stats(&out).copied, 1);
    }

    #[test]
    fn a_frame_carries_the_generation_of_the_resize_that_produced_it_and_is_whole() {
        // A resize mints a new texture, which holds only its fill until
        // something covers it — so the first frame after one must be the whole
        // surface, and must be recognisable as belonging to the new generation.
        let mut driver = one_pane(CONSOLE, PaneKind::Console);
        let mut sink = RecordingSink::new();
        let mut out = Vec::new();
        driver.iterate(&mut sink, &mut out);
        assert_eq!(sink.frames_for(CONSOLE)[0].epoch, 0);

        driver.view_mut(CONSOLE).unwrap().paint = Some(FrameRect::full(WIDTH * 2, HEIGHT));
        driver.apply(
            PaneCommand::Resize {
                id: CONSOLE,
                width: WIDTH * 2,
                height: HEIGHT,
                epoch: 7,
            },
            &mut NoFrameSink,
            &mut out,
        );
        assert_eq!(
            driver.view_mut(CONSOLE).unwrap().resizes,
            vec![(WIDTH * 2, HEIGHT)]
        );

        out.clear();
        driver.iterate(&mut sink, &mut out);
        let frame = sink.frames_for(CONSOLE)[1];
        assert_eq!(frame.epoch, 7, "the generation the resize numbered");
        assert!(frame.full, "and the whole of the new texture");
        assert_eq!(frame.rect, FrameRect::full(WIDTH * 2, HEIGHT));
    }

    #[test]
    fn a_hidden_surface_is_pumped_but_publishes_nothing_and_a_reveal_is_whole() {
        // The point of hiding rather than tearing down: the page stays live and
        // keeps taking state, so a reveal is a `display` flip rather than a page
        // load. What it stops paying is the copy.
        let mut driver = one_pane(HUD, PaneKind::Hud);
        let mut sink = RecordingSink::new();
        let mut out = Vec::new();
        driver.apply(
            PaneCommand::SetHudScript(Some("window.__updateHud({})".to_string())),
            &mut NoFrameSink,
            &mut out,
        );
        driver.apply(
            PaneCommand::SetVisible {
                id: HUD,
                visible: false,
            },
            &mut NoFrameSink,
            &mut out,
        );

        driver.iterate(&mut sink, &mut out);
        driver.iterate(&mut sink, &mut out);
        assert_eq!(
            driver.view_mut(HUD).unwrap().surface.pushed.len(),
            2,
            "the page kept taking state while it was hidden"
        );
        assert!(sink.published.is_empty(), "and cost no copy at all");

        driver.apply(
            PaneCommand::SetVisible {
                id: HUD,
                visible: true,
            },
            &mut NoFrameSink,
            &mut out,
        );
        out.clear();
        driver.iterate(&mut sink, &mut out);
        assert_eq!(sink.published.len(), 1);
        assert!(
            sink.frames_for(HUD)[0].full,
            "the texture is however stale the hidden iterations left it"
        );
        assert_eq!(stats(&out).forced, 1);
    }

    #[test]
    fn a_pane_with_no_buffer_free_is_skipped_and_keeps_what_it_owed() {
        // The render world runs a frame behind, so a pool can genuinely be
        // empty. Allocating a whole surface on the frame path instead would be
        // the wrong answer; Ultralight keeps unioning its dirty bounds until the
        // next successful copy, so the pixels are deferred rather than lost —
        // but only if the force is carried with them.
        let mut driver = one_pane(CONSOLE, PaneKind::Console);
        let mut sink = RecordingSink::new();
        sink.starve = true;
        let mut out = Vec::new();

        driver.iterate(&mut sink, &mut out);
        assert!(sink.published.is_empty());
        assert_eq!(sink.starved, 1);
        assert_eq!(
            driver.view_mut(CONSOLE).unwrap().forced_copies,
            0,
            "the copy was skipped entirely, not made into nothing"
        );

        sink.starve = false;
        out.clear();
        driver.iterate(&mut sink, &mut out);
        assert!(
            sink.frames_for(CONSOLE)[0].full,
            "the whole frame it owed survived the starved iteration"
        );
    }

    #[test]
    fn the_lobby_drains_its_own_bridge_and_a_console_drains_the_bus() {
        // The one thing the two surfaces must not share. Nothing the lobby says
        // is a participant's `ClientMessage` and nothing it hears is a
        // projection, so a lobby that drained the bus — or a console that
        // drained the lobby's queue — would be the whole separation undone.
        let (bus, console) = bus_with_pane();
        let bridge = HostLobbyBridge::new();
        bridge.push_lobby_state("{}");
        broadcast(&bus, ServerMessage::GameStarted);

        let mut driver = PaneLoop::new(RecordingRuntime::default());
        let mut out = Vec::new();
        driver.apply(
            create(console, PaneKind::Console),
            &mut NoFrameSink,
            &mut out,
        );
        driver.apply(create(LOBBY, PaneKind::Lobby), &mut NoFrameSink, &mut out);
        driver.set_bus(Some(bus.clone()));
        driver.set_lobby(Some(bridge.clone()));
        // A page saying something it is not entitled to say is refused and
        // reported rather than swallowed.
        driver
            .view_mut(console)
            .unwrap()
            .surface
            .queue_record(r#"{"type":"Identify","data":{"token":"__local__","name":"impostor"}}"#);

        let mut sink = RecordingSink::new();
        out.clear();
        driver.iterate(&mut sink, &mut out);

        let console_pushed = driver.view_mut(console).unwrap().surface.pushed.clone();
        assert_eq!(console_pushed.len(), 1);
        assert!(console_pushed[0].contains("GameStarted"));
        let lobby_pushed = driver.view_mut(LOBBY).unwrap().surface.pushed.clone();
        assert_eq!(lobby_pushed.len(), 1);
        assert!(
            !lobby_pushed[0].contains("GameStarted"),
            "the bus's traffic never reaches the lobby surface"
        );
        assert!(!bridge.has_pending(), "and the bridge was drained");

        assert!(out
            .iter()
            .any(|e| matches!(e, PaneEvent::Refused { id, .. } if *id == console)));
    }

    #[test]
    fn shutdown_is_the_last_command_the_loop_takes() {
        let mut driver = one_pane(CONSOLE, PaneKind::Console);
        let mut out = Vec::new();
        assert_eq!(
            driver.apply(PaneCommand::Shutdown, &mut NoFrameSink, &mut out),
            LoopControl::Stop
        );
        assert!(out.is_empty());
    }
}
