//! The SDK-independent pane protocol, iteration policy and dedicated thread.
//!
//! [`spawn_pane_thread`] moves only a Send factory onto "phoenix-panes". Its
//! !Send runtime and views are created, driven and dropped there, views first.
//! Commands arrive over std::sync::mpsc; the loop handles them during the wait
//! to its 16 ms deadline as well as before the next iteration. Frames carry
//! their texture epoch and recycle their allocation by drop into a bounded
//! per-pane pool. Replacing a pool on resize also replaces its return channel.
//!
//! Started is the first event, including factory errors and unwind panics.
//! An unwind after startup reports ThreadFailed; release panic=abort cannot
//! recover. The handle sends Shutdown on stop or drop, joins within two seconds
//! when finished, and otherwise detaches without moving SDK objects.
//!
//! # Latest-wins slots, not queued pushes
//!
//! Two of the commands — [`PaneCommand::SetHudScript`] and
//! [`PaneCommand::SetGamepadScript`] — carry a script the producing side wants
//! retained until replaced. They are **slots**: sending one replaces whatever
//! the slot held. A HUD revision is applied once successfully per loaded view,
//! with a fresh application owed after load, reveal or resize. The gamepad
//! snapshot is re-pushed each iteration so the page can poll it on its cadence.
//!
//! The two are pushed at different points of an iteration, and deliberately:
//! the HUD's goes in with the rest of the pumping, *after* `Renderer::update`,
//! so the DOM change it makes is picked up by the `render` below it; the gamepad
//! snapshot goes in *before* `update`, because a console page polls the pads
//! from its own `requestAnimationFrame` callback and that callback is serviced
//! inside `update` — pushing it later would cost a whole iteration of stick
//! latency. See [`PaneLoop::iterate`].
//!
//! The main thread encodes HUD state only when its value changes and sends the
//! new revision to the worker. These slots are bounded by construction. Queued
//! pushes would let a 60 Hz
//! producer outrun a 45 Hz renderer and grow an unbounded backlog of states
//! nobody will ever see — the newest is the only one that matters, and a slot
//! cannot accumulate.
//!
//! # The keys a page receives
//!
//! Backspace, Enter, Escape, the four arrows, Home, End, Delete and a bare Tab
//! are forwarded by `forward_keyboard_text`, each as a
//! `RawKeyDown` with native code `0` and default modifiers. Printable text
//! travels as [`PaneInput::KeyChar`], which is the event that
//! actually puts a character into a field.
//!
//! Enumerating them rather than passing Ultralight's own `VirtualKeyCode`
//! through is what keeps this module SDK-free. Keeping only used keys rather
//! than transcribing the whole table is honesty: a code this side can name but the
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

use std::collections::HashMap;
use std::ops::{Deref, DerefMut};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver, Sender, TryRecvError},
    Arc, Mutex,
};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use super::registry::PaneId;
use super::surface::{pump_pane, PaneSurface, PaneSurfaceError};
use super::surface_stats::{
    elapsed_ns, CopyObservation, FrameTrace, FullCopyReasons, Operation, SurfaceIdentity,
    SurfaceObserver,
};
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
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

/// The editing and dismissal keys forwarded to a page as raw key-downs.
///
/// See the module note for why this contains only forwarded keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaneKeyCode {
    Back,
    Return,
    Escape,
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
    /// The newest HUD readout to retain and apply, or `None` to stop. A **slot**,
    /// not a queued push — see the module note.
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
    /// Per-pane rectangle facts for the opt-in --frame-stats log. The identity
    /// is captured on the worker, so a later resize cannot rename old pixels.
    CopyObserved {
        surface: SurfaceIdentity,
        observation: CopyObservation,
    },
    /// The renderer stopped, and no further event will arrive.
    ThreadFailed { reason: String },
}

/// One renderer iteration's phases and work, excluding the deadline wait.
/// Main-world draining and uploads are measured separately by
/// [`super::frame_stats::PaneFrameSample`]. Every emitted sample is accumulated.
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
    /// more than them for unclassified iteration work; excludes the period wait.
    pub iteration_ms: f64,
    /// Copies deferred for want of a free staging buffer.
    pub starved: usize,
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
    trace: Option<FrameTrace>,
}

impl PaneFrameBuffer {
    /// Take `bytes` for `pane`, to be returned through `recycle` on drop.
    pub fn new(pane: PaneId, bytes: Vec<u8>, recycle: Option<Sender<(PaneId, Vec<u8>)>>) -> Self {
        Self {
            pane,
            bytes,
            recycle,
            trace: None,
        }
    }

    /// Which pane's pool this buffer belongs to.
    pub fn pane(&self) -> PaneId {
        self.pane
    }

    pub fn trace(&self) -> Option<&FrameTrace> {
        self.trace.as_ref()
    }
    pub fn trace_mut(&mut self) -> Option<&mut FrameTrace> {
        self.trace.as_mut()
    }

    /// Attach this production's attribution to its allocation until disposal.
    pub(crate) fn with_trace(mut self, trace: Option<FrameTrace>) -> Self {
        self.trace = trace;
        self
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
        // Record disposal before making the allocation available for a new
        // frame; the trace itself never follows the returned Vec into the pool.
        drop(self.trace.take());
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

    /// The same publication with opt-in accounting. A sink that discards the
    /// observation is recorded as such; only the real pool forwards its lifetime.
    fn publish_observed(
        &mut self,
        id: PaneId,
        epoch: u64,
        rect: FrameRect,
        full: bool,
        trace: Option<FrameTrace>,
    ) {
        self.publish(id, epoch, rect, full);
        drop(trace);
    }

    /// Forget everything held for a pane that has gone.
    fn drop_pane(&mut self, id: PaneId);
}

/// A sink for pure tests of commands that cannot publish a frame. Production
/// lifecycle commands always use the thread's real pool, including Close.
pub struct NoFrameSink;

impl PaneFrameSink for NoFrameSink {
    fn stage(&mut self, _id: PaneId, _len: usize) -> Option<&mut [u8]> {
        None
    }

    fn publish(&mut self, _id: PaneId, _epoch: u64, _rect: FrameRect, _full: bool) {}

    fn drop_pane(&mut self, _id: PaneId) {}
}

/// Measured pipelined-rendering pool: three buffers, at most four returned.
pub const PANE_STAGING_BUFFERS: usize = 3;
pub const PANE_STAGING_CAP: usize = 4;

/// Owned configuration; the runtime is constructed on its owning thread.
pub struct PaneThreadConfig {
    pub bus: Option<PaneBus>,
    pub lobby: Option<HostLobbyBridge>,
    pub period: Duration,
    pub buffers_per_pane: usize,
    pub measure: bool,
    pub observer: Option<SurfaceObserver>,
    /// Optional adapter diagnostic, called on the stopping thread if bounded
    /// shutdown must detach the renderer. Also applies when the handle drops.
    pub on_shutdown_timeout: Option<fn()>,
}
impl Default for PaneThreadConfig {
    fn default() -> Self {
        Self {
            bus: None,
            lobby: None,
            period: Duration::from_millis(16),
            buffers_per_pane: PANE_STAGING_BUFFERS,
            measure: false,
            observer: None,
            on_shutdown_timeout: None,
        }
    }
}
struct FramePool {
    len: usize,
    free: Vec<Vec<u8>>,
    recycle: Sender<(PaneId, Vec<u8>)>,
    returned: Receiver<(PaneId, Vec<u8>)>,
}

/// Thread-owned pools. Resize replaces the return channel too, so equal-length
/// buffers from an old generation cannot return to the new pool.
pub struct PooledFrames {
    pools: HashMap<PaneId, FramePool>,
    staged: Option<(PaneId, Vec<u8>)>,
    frames: Vec<PaneEvent>,
    buffers_per_pane: usize,
    starved: usize,
}
impl PooledFrames {
    pub fn new(buffers_per_pane: usize) -> Self {
        Self {
            pools: HashMap::new(),
            staged: None,
            frames: Vec::new(),
            buffers_per_pane: buffers_per_pane.clamp(1, PANE_STAGING_CAP),
            starved: 0,
        }
    }
    fn configure(&mut self, id: PaneId, kind: PaneKind, len: usize) {
        self.return_staged();
        let (recycle, returned) = mpsc::channel();
        let fill = if kind.transparent() {
            [0, 0, 0, 0]
        } else {
            [0, 0, 0, 255]
        };
        let free = (0..self.buffers_per_pane)
            .map(|_| fill.iter().copied().cycle().take(len).collect())
            .collect();
        self.pools.insert(
            id,
            FramePool {
                len,
                free,
                recycle,
                returned,
            },
        );
    }
    fn return_staged(&mut self) {
        if let Some((id, bytes)) = self.staged.take() {
            if let Some(pool) = self.pools.get_mut(&id) {
                if bytes.len() == pool.len && pool.free.len() < PANE_STAGING_CAP {
                    pool.free.push(bytes);
                }
            }
        }
    }
}
impl PaneFrameSink for PooledFrames {
    fn stage(&mut self, id: PaneId, len: usize) -> Option<&mut [u8]> {
        self.return_staged();
        let pool = self.pools.get_mut(&id)?;
        for (_, bytes) in pool.returned.try_iter() {
            if bytes.len() == pool.len && pool.free.len() < PANE_STAGING_CAP {
                pool.free.push(bytes);
            }
        }
        if len != pool.len {
            return None;
        }
        let Some(bytes) = pool.free.pop() else {
            self.starved += 1;
            return None;
        };
        self.staged = Some((id, bytes));
        self.staged.as_mut().map(|(_, bytes)| bytes.as_mut_slice())
    }
    fn publish(&mut self, id: PaneId, epoch: u64, rect: FrameRect, full: bool) {
        self.publish_observed(id, epoch, rect, full, None);
    }
    fn publish_observed(
        &mut self,
        id: PaneId,
        epoch: u64,
        rect: FrameRect,
        full: bool,
        trace: Option<FrameTrace>,
    ) {
        let Some((staged_id, bytes)) = self.staged.take() else {
            return;
        };
        debug_assert_eq!(id, staged_id);
        let Some(pool) = self.pools.get(&id) else {
            return;
        };
        let buffer = PaneFrameBuffer::new(id, bytes, Some(pool.recycle.clone())).with_trace(trace);
        self.frames.push(PaneEvent::Frame(PaneFrame {
            id,
            epoch,
            rect,
            full,
            bytes: buffer,
        }));
    }
    fn drop_pane(&mut self, id: PaneId) {
        self.return_staged();
        self.pools.remove(&id);
    }
}

/// No renderer or view crosses this thread-safe handle.
pub struct PaneThreadHandle {
    pub commands: Sender<PaneCommand>,
    pub events: Mutex<Receiver<PaneEvent>>,
    finished: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
    on_shutdown_timeout: Option<fn()>,
}
impl PaneThreadHandle {
    pub fn is_running(&self) -> bool {
        !self.finished.load(Ordering::Acquire)
    }
    pub fn send(&self, command: PaneCommand) -> Result<(), mpsc::SendError<PaneCommand>> {
        self.commands.send(command)
    }
    pub fn try_recv(&self) -> Result<PaneEvent, TryRecvError> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .try_recv()
    }
    /// Idempotent bounded shutdown. A stalled renderer stays on its own thread.
    pub fn stop(&mut self) {
        if self.join.is_none() {
            return;
        }
        let _ = self.commands.send(PaneCommand::Shutdown);
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.finished.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        if let Some(join) = self.join.take() {
            if self.finished.load(Ordering::Acquire) {
                let _ = join.join();
            } else {
                // Detaching never drops the renderer or its views on this thread.
                drop(join);
                if let Some(report) = self.on_shutdown_timeout {
                    report();
                }
            }
        }
    }
}
impl Drop for PaneThreadHandle {
    fn drop(&mut self) {
        self.stop();
    }
}
fn panic_reason(payload: Box<dyn std::any::Any + Send>) -> String {
    if let Some(reason) = payload.downcast_ref::<&str>() {
        (*reason).to_owned()
    } else if let Some(reason) = payload.downcast_ref::<String>() {
        reason.clone()
    } else {
        "pane thread panicked".to_owned()
    }
}
fn apply_thread_command<R: PaneRuntime>(
    driver: &mut PaneLoop<R>,
    sink: &mut PooledFrames,
    command: PaneCommand,
    events: &Sender<PaneEvent>,
) -> bool {
    let configure = match &command {
        PaneCommand::Create { id, kind, spec, .. } => {
            Some((*id, *kind, spec.width as usize * spec.height as usize * 4))
        }
        PaneCommand::Resize {
            id, width, height, ..
        } => driver
            .panes
            .iter()
            .find(|p| p.id == *id)
            .map(|p| (*id, p.kind, *width as usize * *height as usize * 4)),
        _ => None,
    };
    let mut out = Vec::new();
    let stop = driver.apply(command, sink, &mut out) == LoopControl::Stop;
    if let Some((id, kind, len)) = configure {
        if driver.contains(id) {
            sink.configure(id, kind, len);
        }
    }
    for event in out {
        if events.send(event).is_err() {
            return false;
        }
    }
    !stop
}

/// Only the factory is Send: a !Send runtime and its views are created, used
/// and destroyed here. Started is always first, even on a factory panic.
/// Unwind recovery applies only in dev builds: release uses panic=abort.
pub fn spawn_pane_thread<R, F>(
    config: PaneThreadConfig,
    start: F,
) -> std::io::Result<PaneThreadHandle>
where
    R: PaneRuntime + 'static,
    F: FnOnce() -> Result<R, String> + Send + 'static,
{
    let (commands, receive) = mpsc::channel();
    let (events, event_rx) = mpsc::channel();
    let finished = Arc::new(AtomicBool::new(false));
    let done = finished.clone();
    let join = thread::Builder::new()
        .name("phoenix-panes".into())
        .spawn(move || {
            struct Finish(Arc<AtomicBool>);
            impl Drop for Finish {
                fn drop(&mut self) {
                    self.0.store(true, Ordering::Release);
                }
            }
            let _finish = Finish(done);
            let mut started = false;
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let runtime = match start() {
                    Ok(runtime) => runtime,
                    Err(reason) => {
                        let _ = events.send(PaneEvent::Started(Err(reason)));
                        return;
                    }
                };
                if events.send(PaneEvent::Started(Ok(()))).is_err() {
                    return;
                }
                started = true;
                let mut driver = PaneLoop::new(runtime);
                driver.set_bus(config.bus);
                driver.set_lobby(config.lobby);
                driver.set_measure(config.measure);
                driver.set_observer(config.observer);
                let mut sink = PooledFrames::new(config.buffers_per_pane);
                let period = config.period.max(Duration::from_millis(1));
                loop {
                    let deadline = Instant::now() + period;
                    loop {
                        match receive.try_recv() {
                            Ok(command) => {
                                if !apply_thread_command(&mut driver, &mut sink, command, &events) {
                                    return;
                                }
                            }
                            Err(TryRecvError::Empty) => break,
                            Err(TryRecvError::Disconnected) => return,
                        }
                        if Instant::now() >= deadline {
                            break;
                        }
                    }
                    let mut out = Vec::new();
                    driver.iterate(&mut sink, &mut out);
                    sink.return_staged();
                    for event in out.iter_mut() {
                        if let PaneEvent::Stats(sample) = event {
                            sample.starved = std::mem::take(&mut sink.starved);
                        }
                    }
                    let sample = out.pop();
                    out.append(&mut sink.frames);
                    out.extend(sample);
                    for event in out {
                        if events.send(event).is_err() {
                            return;
                        }
                    }
                    while let Some(wait) = deadline.checked_duration_since(Instant::now()) {
                        match receive.recv_timeout(wait) {
                            Ok(command) => {
                                if !apply_thread_command(&mut driver, &mut sink, command, &events) {
                                    return;
                                }
                            }
                            Err(mpsc::RecvTimeoutError::Timeout) => break,
                            Err(mpsc::RecvTimeoutError::Disconnected) => return,
                        }
                    }
                }
            }));
            if let Err(payload) = result {
                let reason = panic_reason(payload);
                let _ = events.send(if started {
                    PaneEvent::ThreadFailed { reason }
                } else {
                    PaneEvent::Started(Err(reason))
                });
            }
        })?;
    Ok(PaneThreadHandle {
        commands,
        events: Mutex::new(event_rx),
        finished,
        join: Some(join),
        on_shutdown_timeout: config.on_shutdown_timeout,
    })
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
    device_scale: f64,
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
    full_reasons: FullCopyReasons,
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
    /// The revision this view actually accepted, retained through failed refresh
    /// attempts so produced-frame metadata never claims an unapplied revision.
    applied_hud_revision: Option<u64>,
    /// A lifecycle edge may need the same revision again. Clear only on a
    /// successful push, independently of the copy obligation in `needs_full`.
    hud_apply_owed: bool,
}

impl<V> LoopPane<V> {
    fn identity(&self) -> SurfaceIdentity {
        SurfaceIdentity {
            id: self.id.0,
            epoch: self.epoch,
            kind: match self.kind {
                PaneKind::Console => "console",
                PaneKind::Lobby => "lobby",
                PaneKind::Hud => "hud",
            },
            width: self.size.0,
            height: self.size.1,
            device_scale: self.device_scale,
            visible: self.visible,
        }
    }
}

/// The per-iteration policy: what is driven, in what order, and what comes back.
///
/// This is the whole of what used to be the body of `drive_pane_host`, lifted out of
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
    // Fields drop in declaration order: views must go before their renderer.
    panes: Vec<LoopPane<R::View>>,
    runtime: R,
    /// The pane bus, while there is one. A host with no `--pane` — the lobby
    /// surface alone — has none at all, and its consoles are simply not pumped.
    bus: Option<PaneBus>,
    /// The host lobby's own bridge. Separate from the bus on purpose: nothing
    /// the lobby says is a participant's `ClientMessage`, so nothing it says may
    /// reach the bus.
    lobby: Option<HostLobbyBridge>,
    /// The newest HUD readout, retained for changed revisions and lifecycle
    /// reapplication — a latest-wins slot, see the module note.
    hud_script: Option<String>,
    hud_revision: u64,
    /// The gamepad snapshot to keep pushing into every console, likewise.
    gamepad_script: Option<String>,
    /// Whether to read a clock. Presentation only: an unmeasured host takes no
    /// timestamps at all, and a measured one stamps nothing the simulation can
    /// observe.
    measure: bool,
    observer: Option<SurfaceObserver>,
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
            hud_revision: 0,
            gamepad_script: None,
            measure: false,
            observer: None,
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

    pub fn set_observer(&mut self, observer: Option<SurfaceObserver>) {
        self.observer = observer;
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

    /// One pane's view — **for this crate's tests only** since slice 4.
    ///
    /// Every shipping caller now sends [`PaneCommand::Input`] instead, so no
    /// code outside [`PaneLoop`] reaches a view at all; what is left is the
    /// tests below, which arrange a double's next repaint and read back what it
    /// was given. It stays `pub(crate)` rather than being deleted for that
    /// reason, and cannot widen again: on the day the loop is on its own thread
    /// there is no `&mut` to hand out.
    #[cfg(test)]
    pub(crate) fn view_mut(&mut self, id: PaneId) -> Option<&mut R::View> {
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
        let observer = self.observer.clone();
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
                            device_scale: spec.device_scale,
                            epoch,
                            visible,
                            // A texture that holds only its fill: the first
                            // frame into it must cover the whole surface.
                            needs_full: true,
                            full_reasons: FullCopyReasons {
                                initial: true,
                                ..Default::default()
                            },
                            pushed_this_iteration: false,
                            copy_failures: 0,
                            applied_hud_revision: None,
                            hud_apply_owed: true,
                        });
                        if let Some(observer) = &observer {
                            observer.record(
                                Some(self.panes.last().unwrap().identity()),
                                Operation::Lifecycle { action: "created" },
                            );
                        }
                        Ok(())
                    }
                    Err(e) => Err(e.to_string()),
                };
                out.push(PaneEvent::Created { id, result });
            }
            PaneCommand::Close(id) => {
                if let (Some(observer), Some(pane)) = (&observer, self.pane_mut(id)) {
                    observer.record(
                        Some(pane.identity()),
                        Operation::Lifecycle { action: "closed" },
                    );
                }
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
                    pane.full_reasons.resize = true;
                    pane.hud_apply_owed = true;
                    if let Some(observer) = &observer {
                        observer.record(
                            Some(pane.identity()),
                            Operation::Lifecycle { action: "resized" },
                        );
                    }
                }
            }
            PaneCommand::SetVisible { id, visible } => {
                if let Some(pane) = self.pane_mut(id) {
                    // A surface that was hidden has been publishing nothing
                    // while its page carried on repainting, so its texture is
                    // however stale it is: the reveal owes a whole frame.
                    if visible && !pane.visible {
                        pane.needs_full = true;
                        pane.full_reasons.reveal = true;
                        pane.hud_apply_owed = true;
                    }
                    pane.visible = visible;
                    if let Some(observer) = &observer {
                        observer.record(
                            Some(pane.identity()),
                            Operation::Lifecycle {
                                action: "visibility",
                            },
                        );
                    }
                }
            }
            PaneCommand::Input { id, input } => {
                if let Some(pane) = self.pane_mut(id) {
                    pane.view.input(&input);
                }
            }
            PaneCommand::SetHudScript(script) => {
                // The sender already deduplicates its encoded value. Count each
                // accepted slot replacement, including None. Each view retries
                // it until one application succeeds.
                self.hud_revision = self.hud_revision.wrapping_add(1);
                if let Some(observer) = &observer {
                    observer.record(
                        None,
                        Operation::HudSlot {
                            revision: self.hud_revision,
                            has_script: script.is_some(),
                        },
                    );
                }
                self.hud_script = script;
            }
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
            hud_revision,
            gamepad_script,
            measure,
            observer,
        } = self;
        // Presentation time, never simulation time: an unmeasured loop reads no
        // clock here at all — see the module note in `super::frame_stats`.
        let clock = *measure || observer.is_some();
        let stamp = |on: bool| on.then(Instant::now);
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
                    let started = observer.as_ref().map(|_| Instant::now());
                    let applied = pane.view.push(script).is_ok();
                    if let Some(observer) = &observer {
                        observer.record(
                            Some(pane.identity()),
                            Operation::Push {
                                channel: "gamepad",
                                revision: None,
                                applied: u64::from(applied),
                                failed: u64::from(!applied),
                                deferred_messages: 0,
                                duration_ns: elapsed_ns(started),
                            },
                        );
                    }
                }
            }
        }
        let mut pump_ns = elapsed_ns(phase);

        let phase = stamp(clock);
        runtime.update();
        let update_ns = elapsed_ns(phase);

        let phase = stamp(clock);
        for pane in panes.iter_mut() {
            let pump_started = observer.as_ref().map(|_| Instant::now());
            let mut applied = 0;
            let mut failed = 0;
            let mut deferred_messages = 0;
            pane.pushed_this_iteration = false;
            // A load that has finished is what makes pushes legal. Asking the
            // view each iteration (rather than trusting a callback) keeps this
            // to one place.
            let was_loaded = pane.view.is_ready();
            if pane.view.refresh_loaded() && !was_loaded {
                pane.hud_apply_owed = true;
                out.push(PaneEvent::Loaded(pane.id));
            }
            match pane.kind {
                // The HUD overlay is driven by the host's own readout, held in
                // the slot until replaced. Apply a revision once successfully
                // per loaded view, or again when its lifecycle owes a refresh.
                // This runs before render so the new DOM is painted this pass;
                // a quiet revision leaves animation dirty detection intact.
                PaneKind::Hud => {
                    if let (Some(script), true) = (
                        &*hud_script,
                        pane.view.is_ready()
                            && (pane.hud_apply_owed
                                || pane.applied_hud_revision != Some(*hud_revision)),
                    ) {
                        if pane.view.push(script).is_ok() {
                            pane.pushed_this_iteration = true;
                            pane.applied_hud_revision = Some(*hud_revision);
                            pane.hud_apply_owed = false;
                            applied = 1;
                        } else {
                            failed = 1;
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
                        applied = report.pushed as u64;
                        failed = u64::from(report.push_failure.is_some());
                        deferred_messages = report.deferred as u64;
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
                    applied = report.pushed as u64;
                    failed = u64::from(report.push_failure.is_some());
                    deferred_messages = report.deferred as u64;
                    for refusal in report.refusals {
                        out.push(PaneEvent::Refused {
                            id: pane.id,
                            refusal,
                        });
                    }
                }
            }
            if let Some(observer) = &observer {
                observer.record(
                    Some(pane.identity()),
                    Operation::Push {
                        channel: "bridge_pump",
                        revision: matches!(pane.kind, PaneKind::Hud).then_some(*hud_revision),
                        applied,
                        failed,
                        deferred_messages,
                        duration_ns: elapsed_ns(pump_started),
                    },
                );
            }
        }
        pump_ns += elapsed_ns(phase);

        let phase = stamp(clock);
        runtime.render();
        let render_ns = elapsed_ns(phase);

        let phase = stamp(clock);
        let mut copy_ns = 0u64;
        let mut published_ns = 0u64;
        let mut copied = 0usize;
        let mut forced = 0usize;
        let mut pixels = 0u64;
        for pane in panes.iter_mut() {
            // A hidden surface is pumped but not copied: its page stays live, so
            // a reveal is a `display` flip rather than a page load, and the
            // reveal itself owes the whole frame.
            if !pane.visible {
                if self.measure {
                    out.push(PaneEvent::CopyObserved {
                        surface: pane.identity(),
                        observation: CopyObservation::unobserved(
                            "hidden",
                            false,
                            FullCopyReasons::default(),
                        ),
                    });
                }
                continue;
            }
            // A push we just made is trusted on its own regardless of what the
            // surface reports. `needs_full` carries a force the pane could not
            // honour — a fresh texture, a resize, or an iteration skipped for
            // want of a buffer.
            let force = pane.pushed_this_iteration || pane.needs_full;
            let mut reasons = pane.full_reasons;
            if pane.pushed_this_iteration {
                if matches!(pane.kind, PaneKind::Hud) {
                    reasons.hud_push = true;
                } else {
                    reasons.bridge_push = true;
                }
            }
            let len = pane.size.0 as usize * pane.size.1 as usize * 4;
            let staged = match sink.stage(pane.id, len) {
                Some(buffer) => buffer,
                None => {
                    // No buffer free: skip this pane's copy entirely rather than
                    // allocating a whole surface on the frame path. Ultralight
                    // keeps unioning its dirty bounds until the next successful
                    // copy, so nothing is silently lost — but the frame is.
                    pane.needs_full |= force;
                    if force {
                        pane.full_reasons.merge(reasons);
                        pane.full_reasons.buffer_retry = true;
                    }
                    let observation = CopyObservation::unobserved("buffer_starved", force, reasons);
                    if self.measure {
                        out.push(PaneEvent::CopyObserved {
                            surface: pane.identity(),
                            observation,
                        });
                    }
                    if let Some(observer) = &observer {
                        observer.record(Some(pane.identity()), observation.operation(0));
                    }
                    continue;
                }
            };
            let copy_started = stamp(clock);
            let outcome = pane.view.copy_frame(staged, force);
            let duration_ns = elapsed_ns(copy_started);
            copy_ns += duration_ns;
            let observation = CopyObservation::from_result(&outcome, force, reasons);
            if self.measure {
                out.push(PaneEvent::CopyObserved {
                    surface: pane.identity(),
                    observation,
                });
            }
            if let Some(observer) = &observer {
                observer.record(Some(pane.identity()), observation.operation(duration_ns));
            }
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
                        pane.full_reasons = FullCopyReasons::default();
                        let trace = observer.as_ref().map(|observer| {
                            observer.produced(
                                pane.identity(),
                                rect.pixel_count(),
                                force,
                                reasons,
                                pane.applied_hud_revision,
                            )
                        });
                        let publish_start = observer.as_ref().map(|_| Instant::now());
                        sink.publish_observed(pane.id, pane.epoch, rect, force, trace);
                        published_ns += elapsed_ns(publish_start);
                    }
                    // `Ok(None)` is a still page. The buffer was never filled,
                    // and the sink takes it back unpublished.
                }
                Err(e) => {
                    // The force was not honoured — a push's repaint may not be
                    // in Ultralight's own dirty bounds — so it is carried to the
                    // next attempt.
                    pane.needs_full |= force;
                    if force {
                        pane.full_reasons.merge(reasons);
                        pane.full_reasons.copy_retry = true;
                    }
                    pane.copy_failures += 1;
                    out.push(PaneEvent::CopyFailed {
                        id: pane.id,
                        consecutive: pane.copy_failures,
                        reason: e.to_string(),
                    });
                }
            }
        }
        let publish_ms = elapsed_ns(phase).saturating_sub(copy_ns) as f64 / 1_000_000.0;
        let total_ns = elapsed_ns(iteration);
        if let Some(observer) = &observer {
            observer.record(
                None,
                Operation::Iteration {
                    update_ns,
                    pump_ns,
                    render_ns,
                    copy_ns,
                    publish_ns: published_ns,
                    total_ns,
                },
            );
        }

        out.push(PaneEvent::Stats(PaneThreadSample {
            update_ms: update_ns as f64 / 1_000_000.0,
            pump_ms: pump_ns as f64 / 1_000_000.0,
            render_ms: render_ns as f64 / 1_000_000.0,
            copy_ms: copy_ns as f64 / 1_000_000.0,
            publish_ms,
            panes: panes.len(),
            copied,
            forced,
            pixels,
            iteration_ms: total_ns as f64 / 1_000_000.0,
            starved: 0,
        }));
    }
}

#[cfg(test)]
mod thread_tests {
    use super::doubles::{RecordingRuntime, RecordingView};
    use super::*;

    const PANE: PaneId = PaneId(7);
    const TIMEOUT: Duration = Duration::from_secs(3);

    struct TrackedRuntime {
        inner: RecordingRuntime, // Rc makes this !Send, deliberately.
        owner: thread::ThreadId,
        trace: Sender<(thread::ThreadId, String)>,
    }
    struct TrackedView {
        inner: RecordingView,
        owner: thread::ThreadId,
        trace: Sender<(thread::ThreadId, String)>,
    }
    fn record(owner: thread::ThreadId, trace: &Sender<(thread::ThreadId, String)>, event: String) {
        assert_eq!(owner, thread::current().id(), "SDK call crossed threads");
        let _ = trace.send((thread::current().id(), event));
    }
    impl Drop for TrackedRuntime {
        fn drop(&mut self) {
            record(self.owner, &self.trace, "drop-runtime".into());
        }
    }
    impl Drop for TrackedView {
        fn drop(&mut self) {
            record(self.owner, &self.trace, "drop-view".into());
        }
    }
    impl PaneRuntime for TrackedRuntime {
        type View = TrackedView;
        fn update(&mut self) {
            record(self.owner, &self.trace, "update".into());
            self.inner.update();
        }
        fn render(&mut self) {
            record(self.owner, &self.trace, "render".into());
            self.inner.render();
        }
        fn create(
            &mut self,
            id: PaneId,
            kind: PaneKind,
            spec: &PaneSpecOwned,
            url: &str,
        ) -> Result<Self::View, PaneSurfaceError> {
            record(self.owner, &self.trace, "create".into());
            let mut inner = self.inner.create(id, kind, spec, url)?;
            inner.paint = Some(FrameRect::full(spec.width, spec.height));
            Ok(TrackedView {
                inner,
                owner: self.owner,
                trace: self.trace.clone(),
            })
        }
    }
    impl PaneSurface for TrackedView {
        fn load(&mut self, url: &str) -> Result<(), PaneSurfaceError> {
            self.inner.load(url)
        }
        fn is_ready(&self) -> bool {
            self.inner.is_ready()
        }
        fn push(&mut self, script: &str) -> Result<(), PaneSurfaceError> {
            record(self.owner, &self.trace, format!("push:{script}"));
            self.inner.push(script)
        }
        fn drain(&mut self) -> Vec<String> {
            self.inner.drain()
        }
    }
    impl PaneView for TrackedView {
        fn refresh_loaded(&mut self) -> bool {
            self.inner.refresh_loaded()
        }
        fn resize(&mut self, width: u32, height: u32) {
            record(self.owner, &self.trace, "resize".into());
            self.inner.resize(width, height);
            self.inner.paint = Some(FrameRect::full(width, height));
        }
        fn input(&mut self, input: &PaneInput) {
            record(self.owner, &self.trace, format!("input:{input:?}"));
            assert!(
                !matches!(input, PaneInput::KeyChar(text) if text == "panic"),
                "view panic"
            );
            self.inner.input(input);
        }
        fn copy_frame(
            &mut self,
            dst: &mut [u8],
            force: bool,
        ) -> Result<Option<FrameRect>, PaneSurfaceError> {
            record(self.owner, &self.trace, "copy".into());
            self.inner.copy_frame(dst, force)
        }
    }
    fn spawn(period: Duration) -> (PaneThreadHandle, Receiver<(thread::ThreadId, String)>) {
        let (trace, observed) = mpsc::channel();
        let handle = spawn_pane_thread(
            PaneThreadConfig {
                period,
                measure: true,
                ..Default::default()
            },
            move || {
                let owner = thread::current().id();
                assert_eq!(thread::current().name(), Some("phoenix-panes"));
                record(owner, &trace, "start".into());
                Ok(TrackedRuntime {
                    inner: RecordingRuntime::default(),
                    owner,
                    trace,
                })
            },
        )
        .unwrap();
        (handle, observed)
    }
    fn event(handle: &PaneThreadHandle) -> PaneEvent {
        handle
            .events
            .lock()
            .unwrap()
            .recv_timeout(TIMEOUT)
            .expect("pane thread event")
    }
    fn until(handle: &PaneThreadHandle, predicate: impl Fn(&PaneEvent) -> bool) -> PaneEvent {
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            let event = handle
                .events
                .lock()
                .unwrap()
                .recv_timeout(remaining)
                .expect("expected pane event");
            if predicate(&event) {
                return event;
            }
        }
    }
    fn create() -> PaneCommand {
        PaneCommand::Create {
            id: PANE,
            kind: PaneKind::Console,
            spec: PaneSpecOwned {
                width: 2,
                height: 2,
                device_scale: 1.0,
            },
            url: "http://localhost/console".into(),
            epoch: 0,
            visible: true,
        }
    }

    #[test]
    fn real_thread_keeps_a_non_send_runtime_and_views_on_one_thread_and_drops_views_first() {
        fn send_sync<T: Send + Sync>() {}
        send_sync::<PaneThreadHandle>();
        let main = thread::current().id();
        let (mut handle, trace) = spawn(Duration::from_millis(5));
        assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
        assert!(handle.is_running());
        handle.send(create()).unwrap();
        until(&handle, |event| matches!(event, PaneEvent::Frame(_)));
        handle
            .send(PaneCommand::Input {
                id: PANE,
                input: PaneInput::Focus,
            })
            .unwrap();
        handle
            .send(PaneCommand::Input {
                id: PANE,
                input: PaneInput::KeyChar("a".into()),
            })
            .unwrap();
        handle.stop();
        handle.stop();
        assert!(!handle.is_running());
        let observed: Vec<_> = trace.try_iter().collect();
        assert!(observed.iter().all(|(owner, _)| *owner != main));
        let names: Vec<_> = observed.iter().map(|(_, event)| event.as_str()).collect();
        let focus = names
            .iter()
            .position(|name| *name == "input:Focus")
            .unwrap();
        let key = names
            .iter()
            .position(|name| *name == "input:KeyChar(\"a\")")
            .unwrap();
        assert!(focus < key);
        assert_eq!(&names[names.len() - 2..], &["drop-view", "drop-runtime"]);
    }

    #[test]
    fn a_command_arriving_during_the_wait_is_applied_before_the_next_iteration() {
        let (mut handle, _) = spawn(Duration::from_secs(1));
        assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
        until(&handle, |event| matches!(event, PaneEvent::Stats(_)));
        handle.send(create()).unwrap();
        let created = handle
            .events
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_millis(500))
            .unwrap();
        assert!(matches!(created, PaneEvent::Created { result: Ok(()), .. }));
        handle.stop();
    }

    #[test]
    fn startup_failure_and_startup_panic_both_begin_with_a_failed_handshake() {
        for panic in [false, true] {
            let mut handle = spawn_pane_thread(PaneThreadConfig::default(), move || {
                assert!(!panic, "factory panic");
                Err::<RecordingRuntime, _>("SDK unavailable".into())
            })
            .unwrap();
            assert!(matches!(event(&handle), PaneEvent::Started(Err(_))));
            handle.stop();
            assert!(!handle.is_running());
            assert!(matches!(handle.try_recv(), Err(TryRecvError::Disconnected)));
        }
    }

    #[test]
    fn an_unwinding_view_failure_reports_terminal_death_and_finishes() {
        let (mut handle, trace) = spawn(Duration::from_millis(5));
        assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
        handle.send(create()).unwrap();
        until(&handle, |event| matches!(event, PaneEvent::Created { .. }));
        handle
            .send(PaneCommand::Input {
                id: PANE,
                input: PaneInput::KeyChar("panic".into()),
            })
            .unwrap();
        until(
            &handle,
            |event| matches!(event, PaneEvent::ThreadFailed { reason } if reason == "view panic"),
        );
        handle.stop();
        let names: Vec<_> = trace.try_iter().map(|(_, name)| name).collect();
        assert_eq!(&names[names.len() - 2..], &["drop-view", "drop-runtime"]);
    }

    #[test]
    fn a_failed_create_is_reported_without_stopping_other_seats() {
        let mut handle = spawn_pane_thread(PaneThreadConfig::default(), || {
            Ok(RecordingRuntime {
                fail_create: HashMap::from([(PANE, "refused".into())]),
                ..Default::default()
            })
        })
        .unwrap();
        assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
        handle.send(create()).unwrap();
        until(
            &handle,
            |event| matches!(event, PaneEvent::Created { id, result: Err(_) } if *id == PANE),
        );
        let mut other = create();
        if let PaneCommand::Create { id, .. } = &mut other {
            *id = PaneId(8);
        }
        handle.send(other).unwrap();
        until(
            &handle,
            |event| matches!(event, PaneEvent::Created { id, result: Ok(()) } if *id == PaneId(8)),
        );
        assert!(handle.is_running());
        handle.stop();
    }

    #[test]
    fn dropping_the_handle_stops_the_owning_thread() {
        let (handle, trace) = spawn(Duration::from_millis(5));
        assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
        drop(handle);
        assert!(trace.try_iter().any(|(_, name)| name == "drop-runtime"));
    }

    #[test]
    fn a_stalled_thread_reports_timeout_once_and_drops_its_runtime_on_the_owner() {
        static REPORTS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let (blocked, waiting) = mpsc::channel();
        let (release, resume) = mpsc::channel();
        let (trace, observed) = mpsc::channel();
        let main = thread::current().id();
        let mut handle = spawn_pane_thread(
            PaneThreadConfig {
                on_shutdown_timeout: Some(|| {
                    REPORTS.fetch_add(1, Ordering::Relaxed);
                }),
                ..Default::default()
            },
            move || {
                let runtime = TrackedRuntime {
                    inner: RecordingRuntime::default(),
                    owner: thread::current().id(),
                    trace,
                };
                blocked.send(()).unwrap();
                resume.recv_timeout(Duration::from_secs(10)).unwrap();
                Ok(runtime)
            },
        )
        .unwrap();
        waiting.recv_timeout(TIMEOUT).unwrap();

        let start = Instant::now();
        handle.stop();
        assert!(start.elapsed() < TIMEOUT, "shutdown must stay bounded");
        assert!(
            handle.is_running(),
            "the stalled runtime remains on its owner"
        );
        assert_eq!(REPORTS.load(Ordering::Relaxed), 1);
        handle.stop();
        drop(handle);
        assert_eq!(REPORTS.load(Ordering::Relaxed), 1);

        release.send(()).unwrap();
        let (owner, event) = observed.recv_timeout(TIMEOUT).unwrap();
        assert_ne!(owner, main);
        assert_eq!(event, "drop-runtime");
    }

    #[test]
    fn resize_changes_the_frame_generation_and_close_stops_publication() {
        let (mut handle, _) = spawn(Duration::from_millis(5));
        assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
        handle.send(create()).unwrap();
        until(&handle, |event| matches!(event, PaneEvent::Frame(_)));
        handle
            .send(PaneCommand::Resize {
                id: PANE,
                width: 3,
                height: 2,
                epoch: 1,
            })
            .unwrap();
        let PaneEvent::Frame(frame) = until(
            &handle,
            |event| matches!(event, PaneEvent::Frame(frame) if frame.epoch == 1),
        ) else {
            unreachable!()
        };
        assert_eq!(frame.bytes.len(), 24);
        assert_eq!(frame.rect, FrameRect::full(3, 2));
        assert!(frame.full);
        drop(frame);
        handle.send(PaneCommand::Close(PANE)).unwrap();
        until(
            &handle,
            |event| matches!(event, PaneEvent::Stats(sample) if sample.panes == 0),
        );
        assert!(matches!(event(&handle), PaneEvent::Stats(sample) if sample.panes == 0));
        handle.stop();
    }

    #[test]
    fn retained_empty_gamepad_slot_reaches_js_even_when_main_frames_outrun_iterations() {
        let (mut handle, trace) = spawn(Duration::from_millis(5));
        assert!(matches!(event(&handle), PaneEvent::Started(Ok(()))));
        handle.send(create()).unwrap();
        until(&handle, |event| matches!(event, PaneEvent::Frame(_)));
        handle
            .send(PaneCommand::SetGamepadScript(Some("populated".into())))
            .unwrap();
        handle
            .send(PaneCommand::SetGamepadScript(Some("empty".into())))
            .unwrap();
        // Quiet main frames send no None, so the final state cannot be coalesced away.
        for _ in 0..3 {
            until(&handle, |event| matches!(event, PaneEvent::Stats(_)));
        }
        handle.stop();
        let names: Vec<_> = trace.try_iter().map(|(_, name)| name).collect();
        let first = names.iter().position(|name| name == "push:empty").unwrap();
        assert_eq!(names[first + 1], "update");
        assert!(names.iter().filter(|name| *name == "push:empty").count() >= 2);
    }

    #[test]
    fn pooled_frames_are_bounded_recycled_and_isolated_across_resize_and_close() {
        let mut sink = PooledFrames::new(PANE_STAGING_BUFFERS);
        sink.configure(PANE, PaneKind::Console, 16);
        for _ in 0..3 {
            assert_eq!(sink.stage(PANE, 16).unwrap()[3], 255);
            sink.publish(PANE, 0, FrameRect::full(2, 2), true);
        }
        assert!(sink.stage(PANE, 16).is_none());
        let held = sink.frames.pop().unwrap();
        drop(held);
        assert!(
            sink.stage(PANE, 16).is_some(),
            "drop recycled one allocation"
        );
        sink.publish(PANE, 0, FrameRect::full(2, 2), true);
        let old_frames = std::mem::take(&mut sink.frames);
        sink.configure(PANE, PaneKind::Hud, 16); // Equal length, different generation.
        drop(old_frames);
        for _ in 0..3 {
            assert_eq!(sink.stage(PANE, 16).unwrap()[3], 0);
            sink.publish(PANE, 1, FrameRect::full(2, 2), true);
        }
        assert!(
            sink.stage(PANE, 16).is_none(),
            "old buffers never inflated this pool"
        );
        sink.drop_pane(PANE);
        sink.frames.clear();
        assert!(
            sink.stage(PANE, 16).is_none(),
            "late returns never resurrect a closed pane"
        );
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

    /// How one input names itself in the shared phase log.
    fn input_kind(input: &PaneInput) -> &'static str {
        match input {
            PaneInput::MouseMove { .. } => "mousemove",
            PaneInput::MouseDown { .. } => "mousedown",
            PaneInput::MouseUp { .. } => "mouseup",
            PaneInput::Scroll { .. } => "scroll",
            PaneInput::Key(_) => "key",
            PaneInput::KeyChar(_) => "keychar",
            PaneInput::Focus => "focus",
            PaneInput::Unfocus => "unfocus",
        }
    }

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
            self.trace("resize");
            self.resizes.push((width, height));
        }

        fn input(&mut self, input: &PaneInput) {
            // `input:<pane>:<kind>` rather than the usual `<what>:<pane>`: a
            // queue's claim is about the order of one pane's stream, so which
            // input it was has to be in the line.
            if let Some(trace) = &self.trace {
                trace
                    .borrow_mut()
                    .push(format!("input:{}:{}", self.label, input_kind(input)));
            }
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
    //! Every claim here used to be a claim about `drive_pane_host`, provable only by
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
    fn disabled_attribution_attaches_no_trace_or_phase_clock_samples() {
        let mut driver = one_pane(HUD, PaneKind::Hud);
        let mut sink = PooledFrames::new(PANE_STAGING_BUFFERS);
        sink.configure(HUD, PaneKind::Hud, (WIDTH * HEIGHT * 4) as usize);
        let mut out = Vec::new();
        driver.iterate(&mut sink, &mut out);
        let PaneEvent::Frame(frame) = &sink.frames[0] else {
            panic!("a real frame")
        };
        assert!(frame.bytes.trace().is_none());
        assert!(
            !out.iter()
                .any(|event| matches!(event, PaneEvent::CopyObserved { .. })),
            "the ordinary unmeasured path sends no per-pane log events"
        );
        let sample = stats(&out);
        assert_eq!(
            [
                sample.update_ms,
                sample.pump_ms,
                sample.render_ms,
                sample.copy_ms,
                sample.publish_ms,
                sample.iteration_ms
            ],
            [0.0; 6]
        );
    }

    #[test]
    fn measured_rectangle_events_follow_real_copy_retry_hidden_and_resize_decisions() {
        fn step(
            driver: &mut PaneLoop<RecordingRuntime>,
            sink: &mut RecordingSink,
        ) -> (SurfaceIdentity, CopyObservation, PaneThreadSample) {
            let mut out = Vec::new();
            driver.iterate(sink, &mut out);
            let observations: Vec<_> = out
                .iter()
                .filter_map(|event| match event {
                    PaneEvent::CopyObserved {
                        surface,
                        observation,
                    } => Some((*surface, *observation)),
                    _ => None,
                })
                .collect();
            assert_eq!(
                observations.len(),
                1,
                "one decision per measured pane iteration"
            );
            (observations[0].0, observations[0].1, stats(&out))
        }
        let observer = SurfaceObserver::new(Instant::now(), 128);
        let mut driver = one_pane(CONSOLE, PaneKind::Console);
        driver.set_measure(true);
        driver.set_observer(Some(observer.clone()));
        let mut sink = RecordingSink::new();
        let (old_identity, initial, _) = step(&mut driver, &mut sink);
        assert!(initial.forced && initial.reasons.initial);
        assert_eq!(initial.dirty_rect, None);
        assert_eq!(initial.copied_rect, Some(FrameRect::full(WIDTH, HEIGHT)));

        let partial = FrameRect {
            left: 1,
            top: 0,
            right: 3,
            bottom: 1,
        };
        driver.view_mut(CONSOLE).unwrap().paint = Some(partial);
        let (_, copied, sample) = step(&mut driver, &mut sink);
        assert_eq!(
            (copied.dirty_rect, copied.copied_rect),
            (Some(partial), Some(partial))
        );
        assert_eq!(sample.copied, 1);
        assert!(!copied.forced);
        driver.view_mut(CONSOLE).unwrap().paint = None;
        let (_, clean, sample) = step(&mut driver, &mut sink);
        assert_eq!(clean.dirty_rect, Some(FrameRect::default()));
        assert_eq!(sample.copied, 0);

        // A failed unforced copy does not create a new full-copy obligation.
        // A real reveal does; exercise its preservation through both failures.
        for visible in [false, true] {
            driver.apply(
                PaneCommand::SetVisible {
                    id: CONSOLE,
                    visible,
                },
                &mut sink,
                &mut Vec::new(),
            );
        }
        driver.view_mut(CONSOLE).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));
        driver.view_mut(CONSOLE).unwrap().fail_copies = 1;
        let (_, failed, _) = step(&mut driver, &mut sink);
        assert!(failed.forced && failed.reasons.reveal);
        assert_eq!(
            (failed.outcome, failed.dirty_rect, failed.copied_rect),
            ("failed", None, None)
        );
        sink.starve = true;
        let (_, starved, _) = step(&mut driver, &mut sink);
        assert_eq!(
            (starved.outcome, starved.dirty_rect, starved.copied_rect),
            ("buffer_starved", None, None)
        );
        assert!(starved.forced && starved.reasons.copy_retry);
        sink.starve = false;
        let (_, retried, _) = step(&mut driver, &mut sink);
        assert!(retried.forced && retried.reasons.copy_retry && retried.reasons.buffer_retry);
        assert_eq!(retried.dirty_rect, None);

        driver.apply(
            PaneCommand::SetVisible {
                id: CONSOLE,
                visible: false,
            },
            &mut sink,
            &mut Vec::new(),
        );
        let before = sink.published.len();
        let (hidden_identity, hidden, sample) = step(&mut driver, &mut sink);
        assert!(!hidden_identity.visible);
        assert_eq!(
            (hidden.outcome, hidden.dirty_rect, hidden.copied_rect),
            ("hidden", None, None)
        );
        assert_eq!(sample.copied, 0);
        assert_eq!(sink.published.len(), before);
        driver.apply(
            PaneCommand::Resize {
                id: CONSOLE,
                width: 8,
                height: 6,
                epoch: 3,
            },
            &mut sink,
            &mut Vec::new(),
        );
        driver.apply(
            PaneCommand::SetVisible {
                id: CONSOLE,
                visible: true,
            },
            &mut sink,
            &mut Vec::new(),
        );
        driver.view_mut(CONSOLE).unwrap().paint = Some(FrameRect::full(8, 6));
        let (resized, reveal, _) = step(&mut driver, &mut sink);
        assert_eq!(
            (
                resized.epoch,
                resized.width,
                resized.height,
                resized.visible
            ),
            (3, 8, 6, true)
        );
        assert!(reveal.forced && reveal.reasons.resize && reveal.reasons.reveal);
        assert_eq!(
            (reveal.dirty_rect, reveal.copied_rect),
            (None, Some(FrameRect::full(8, 6)))
        );
        assert_eq!(driver.view_mut(CONSOLE).unwrap().resizes, [(8, 6)]);
        // Already queued observations keep the old raster/epoch after resize.
        assert_eq!(
            (old_identity.epoch, old_identity.width, old_identity.height),
            (0, WIDTH, HEIGHT)
        );
        let copies: Vec<_> = observer
            .events()
            .into_iter()
            .filter(|event| matches!(event.operation, Operation::Copy { .. }))
            .collect();
        assert_eq!(
            copies.len(),
            7,
            "a hidden view never calls or records a copy"
        );
        assert!(
            matches!(copies[1].operation, Operation::Copy { dirty_rect: Some(rect), copied_rect: Some(copied), dirty_pixels: Some(2), copied_pixels: 2, .. } if rect == partial && copied == partial)
        );
    }

    #[test]
    fn attribution_distinguishes_quiet_hud_revisions_hidden_failure_and_reveal() {
        let observer = SurfaceObserver::new(Instant::now(), 128);
        let mut driver = one_pane(HUD, PaneKind::Hud);
        driver.set_observer(Some(observer.clone()));
        let mut sink = RecordingSink::new();
        let mut out = Vec::new();
        driver.apply(
            PaneCommand::SetHudScript(Some("first".into())),
            &mut sink,
            &mut out,
        );
        driver.iterate(&mut sink, &mut out);
        driver.iterate(&mut sink, &mut out);
        driver.apply(
            PaneCommand::SetVisible {
                id: HUD,
                visible: false,
            },
            &mut sink,
            &mut out,
        );
        driver.iterate(&mut sink, &mut out);
        driver.apply(
            PaneCommand::SetHudScript(Some("second".into())),
            &mut sink,
            &mut out,
        );
        driver.view_mut(HUD).unwrap().surface.failing_pushes = 1;
        driver.iterate(&mut sink, &mut out);
        driver.apply(
            PaneCommand::SetVisible {
                id: HUD,
                visible: true,
            },
            &mut sink,
            &mut out,
        );
        driver.iterate(&mut sink, &mut out);

        let events = observer.events();
        let applications: Vec<_> = events
            .iter()
            .filter_map(|event| match event.operation {
                Operation::Push {
                    revision: Some(revision),
                    applied,
                    failed,
                    ..
                } => Some((revision, applied, failed, event.surface.unwrap().visible)),
                _ => None,
            })
            .collect();
        assert_eq!(
            applications,
            vec![
                (1, 1, 0, true),
                (1, 0, 0, true),
                (1, 0, 0, false),
                (2, 0, 1, false),
                (2, 1, 0, true)
            ]
        );
        let produced: Vec<_> = events
            .iter()
            .filter_map(|event| match event.operation {
                Operation::Produced {
                    hud_revision,
                    reasons,
                    ..
                } => Some((hud_revision, reasons)),
                _ => None,
            })
            .collect();
        assert_eq!(
            produced.len(),
            3,
            "hidden views still pump but publish no pixels"
        );
        assert!(produced[0].1.initial && produced[0].1.hud_push);
        assert_eq!(produced[1].0, Some(1));
        assert!(!produced[1].1.hud_push, "unchanged HUD is not reapplied");
        assert!(produced[2].1.reveal && produced[2].1.hud_push);
        assert_eq!(produced[2].0, Some(2));
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e.operation, Operation::Iteration { .. }))
                .count(),
            5
        );
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e.operation, Operation::HudSlot { .. }))
                .count(),
            2
        );
    }

    #[test]
    fn attribution_distinguishes_failed_copy_and_starvation_from_produced_frames() {
        let observer = SurfaceObserver::new(Instant::now(), 64);
        let mut driver = one_pane(HUD, PaneKind::Hud);
        driver.set_observer(Some(observer.clone()));
        let mut sink = RecordingSink::new();
        let mut out = Vec::new();
        driver.view_mut(HUD).unwrap().fail_copies = 1;
        driver.iterate(&mut sink, &mut out);
        sink.starve = true;
        driver.iterate(&mut sink, &mut out);
        sink.starve = false;
        driver.iterate(&mut sink, &mut out);
        let events = observer.events();
        let outcomes: Vec<_> = events
            .iter()
            .filter_map(|event| match event.operation {
                Operation::Copy { outcome, .. } => Some(outcome),
                _ => None,
            })
            .collect();
        assert_eq!(outcomes, ["failed", "buffer_starved", "copied"]);
        let produced: Vec<_> = events
            .iter()
            .filter_map(|event| match event.operation {
                Operation::Produced { reasons, .. } => Some(reasons),
                _ => None,
            })
            .collect();
        assert_eq!(produced.len(), 1);
        assert!(produced[0].initial && produced[0].copy_retry && produced[0].buffer_retry);
    }

    #[test]
    fn observed_pool_preserves_old_frame_identity_across_resize_and_recycles_once() {
        let observer = SurfaceObserver::new(Instant::now(), 128);
        let mut driver = one_pane(HUD, PaneKind::Hud);
        driver.set_observer(Some(observer.clone()));
        let mut sink = PooledFrames::new(PANE_STAGING_BUFFERS);
        sink.configure(HUD, PaneKind::Hud, (WIDTH * HEIGHT * 4) as usize);
        let mut out = Vec::new();
        for _ in 0..PANE_STAGING_BUFFERS + 1 {
            driver.iterate(&mut sink, &mut out);
        }
        assert_eq!(sink.frames.len(), PANE_STAGING_BUFFERS);
        assert_eq!(sink.starved, 1);
        let old_frames = std::mem::take(&mut sink.frames);
        driver.apply(
            PaneCommand::Resize {
                id: HUD,
                width: WIDTH * 2,
                height: HEIGHT,
                epoch: 7,
            },
            &mut sink,
            &mut out,
        );
        sink.configure(HUD, PaneKind::Hud, (WIDTH * 2 * HEIGHT * 4) as usize);
        driver.view_mut(HUD).unwrap().paint = Some(FrameRect::full(WIDTH * 2, HEIGHT));
        driver.iterate(&mut sink, &mut out);
        drop(old_frames);
        sink.frames.clear();
        let events = observer.events();
        let terminal: Vec<_> = events
            .iter()
            .filter(|event| matches!(event.operation, Operation::Discarded { .. }))
            .collect();
        assert_eq!(terminal.len(), PANE_STAGING_BUFFERS + 1);
        assert!(terminal[..PANE_STAGING_BUFFERS].iter().all(|event| {
            let identity = event.surface.unwrap();
            identity.epoch == 0 && identity.width == WIDTH && identity.device_scale == 1.0
        }));
        assert_eq!(terminal.last().unwrap().surface.unwrap().epoch, 7);
        // The old generation returns to its retired channel, never the new pool.
        assert!(sink.stage(HUD, (WIDTH * 2 * HEIGHT * 4) as usize).is_some());
        sink.return_staged();
        assert_eq!(sink.pools[&HUD].free.len(), PANE_STAGING_BUFFERS);
        assert_eq!(
            observer.events().len(),
            events.len(),
            "recycling does not duplicate disposal"
        );
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
        driver.apply(
            PaneCommand::SetHudScript(Some("window.__updateHud({heading:1})".to_string())),
            &mut NoFrameSink,
            &mut out,
        );
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
            vec![true, false, false],
            "the first update is whole; unchanged and cleared HUD slots do not force copying"
        );
        assert!(
            frames.iter().all(|f| f.first_byte == 0xAB),
            "every published buffer carries what the copy wrote"
        );
        assert_eq!(stats(&out).copied, 1);
        assert_eq!(stats(&out).forced, 0);
    }

    #[test]
    fn quiet_hud_keeps_animation_dirty_copies_and_retries_without_reapplying() {
        let observer = SurfaceObserver::new(Instant::now(), 256);
        let mut driver = one_pane(HUD, PaneKind::Hud);
        driver.set_observer(Some(observer.clone()));
        let mut sink = RecordingSink::new();
        let mut out = Vec::new();
        driver.apply(
            PaneCommand::SetHudScript(Some("first".into())),
            &mut sink,
            &mut out,
        );
        driver.iterate(&mut sink, &mut out);
        driver.view_mut(HUD).unwrap().paint = None;
        for _ in 0..3 {
            driver.iterate(&mut sink, &mut out);
        }
        assert_eq!(driver.view_mut(HUD).unwrap().surface.pushed, ["first"]);
        assert_eq!(
            sink.published.len(),
            1,
            "a static HUD produces no more frames"
        );
        assert_eq!(driver.view_mut(HUD).unwrap().forced_copies, 1);

        // A CSS animation can repaint without a new HUD revision. The ordinary
        // render/copy pass still sees that rectangle and does not force it full.
        let animated = FrameRect {
            left: 1,
            top: 0,
            right: 3,
            bottom: 1,
        };
        driver.view_mut(HUD).unwrap().paint = Some(animated);
        driver.iterate(&mut sink, &mut out);
        assert_eq!(sink.published[1].rect, animated);
        assert!(!sink.published[1].full);

        driver.apply(
            PaneCommand::SetHudScript(Some("second".into())),
            &mut sink,
            &mut out,
        );
        driver.view_mut(HUD).unwrap().surface.failing_pushes = 1;
        driver.view_mut(HUD).unwrap().paint = None;
        driver.iterate(&mut sink, &mut out);
        assert_eq!(driver.pane_mut(HUD).unwrap().applied_hud_revision, Some(1));

        // A successful retry owes a whole copy even if no staging buffer is
        // free, and then the copy itself fails. No further HUD push is needed
        // for that same obligation to reach the eventual successful frame.
        driver.view_mut(HUD).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));
        sink.starve = true;
        driver.iterate(&mut sink, &mut out);
        assert_eq!(driver.pane_mut(HUD).unwrap().applied_hud_revision, Some(2));
        sink.starve = false;
        driver.view_mut(HUD).unwrap().fail_copies = 1;
        driver.iterate(&mut sink, &mut out);
        driver.iterate(&mut sink, &mut out);
        assert_eq!(
            driver.view_mut(HUD).unwrap().surface.pushed,
            ["first", "second"]
        );
        assert_eq!(sink.published.len(), 3);
        assert!(sink.published[2].full);
        let events = observer.events();
        let (revision, reasons) = events
            .iter()
            .rev()
            .find_map(|event| match event.operation {
                Operation::Produced {
                    hud_revision,
                    reasons,
                    ..
                } => Some((hud_revision, reasons)),
                _ => None,
            })
            .expect("the retry produced a frame");
        assert_eq!(revision, Some(2));
        assert!(reasons.hud_push && reasons.buffer_retry && reasons.copy_retry);
    }

    #[test]
    fn latest_hud_survives_loading_recreation_reveal_and_resize() {
        let mut driver = one_pane(HUD, PaneKind::Hud);
        driver.view_mut(HUD).unwrap().finishes_loading_after = 1;
        let mut sink = RecordingSink::new();
        let mut out = Vec::new();
        driver.apply(
            PaneCommand::SetHudScript(Some("before load".into())),
            &mut sink,
            &mut out,
        );
        driver.iterate(&mut sink, &mut out);
        assert!(driver.view_mut(HUD).unwrap().surface.pushed.is_empty());
        driver.apply(
            PaneCommand::SetHudScript(Some("latest".into())),
            &mut sink,
            &mut out,
        );
        driver.iterate(&mut sink, &mut out);
        assert_eq!(driver.view_mut(HUD).unwrap().surface.pushed, ["latest"]);

        driver.apply(PaneCommand::Close(HUD), &mut sink, &mut out);
        driver.apply(create(HUD, PaneKind::Hud), &mut sink, &mut out);
        driver.view_mut(HUD).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));
        driver.iterate(&mut sink, &mut out);
        assert_eq!(driver.view_mut(HUD).unwrap().surface.pushed, ["latest"]);
        for visible in [false, true] {
            driver.apply(
                PaneCommand::SetVisible { id: HUD, visible },
                &mut sink,
                &mut out,
            );
            driver.iterate(&mut sink, &mut out);
        }
        assert_eq!(
            driver.view_mut(HUD).unwrap().surface.pushed,
            ["latest", "latest"]
        );
        driver.apply(
            PaneCommand::Resize {
                id: HUD,
                width: WIDTH * 2,
                height: HEIGHT,
                epoch: 7,
            },
            &mut sink,
            &mut out,
        );
        driver.view_mut(HUD).unwrap().paint = Some(FrameRect::full(WIDTH * 2, HEIGHT));
        driver.view_mut(HUD).unwrap().surface.failing_pushes = 1;
        driver.iterate(&mut sink, &mut out);
        assert_eq!(driver.pane_mut(HUD).unwrap().applied_hud_revision, Some(2));
        assert!(driver.pane_mut(HUD).unwrap().hud_apply_owed);
        driver.iterate(&mut sink, &mut out);
        assert!(!driver.pane_mut(HUD).unwrap().hud_apply_owed);
        assert_eq!(
            driver.view_mut(HUD).unwrap().surface.pushed,
            ["latest", "latest", "latest"]
        );
        let frame = sink.published.last().unwrap();
        assert_eq!(frame.epoch, 7);
        assert_eq!(frame.rect, FrameRect::full(WIDTH * 2, HEIGHT));
        assert!(
            frame.full,
            "the retried resize application is painted whole"
        );
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
        driver.apply(
            PaneCommand::SetHudScript(Some("window.__updateHud({heading:1})".to_string())),
            &mut NoFrameSink,
            &mut out,
        );
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
        super::super::transport::identify_test_pane(&bus, console);
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

    /// Drain a queue of commands the way `drive_pane_host` does: FIFO, through
    /// [`PaneLoop::apply`], before the iteration.
    fn drain(
        driver: &mut PaneLoop<RecordingRuntime>,
        queue: &mut std::collections::VecDeque<PaneCommand>,
        out: &mut Vec<PaneEvent>,
    ) {
        while let Some(cmd) = queue.pop_front() {
            assert_eq!(
                driver.apply(cmd, &mut NoFrameSink, out),
                LoopControl::Continue,
                "nothing a Bevy system queues stops the loop"
            );
        }
    }

    #[test]
    fn a_drained_queue_delivers_its_inputs_in_order_and_before_the_iteration() {
        // Slice 4's whole claim: a queue between the system and the view changes
        // WHERE the call is made, not when it lands nor in what order. The
        // systems that fill it are chained ahead of `drive_pane_host`, so every
        // command raised in a frame is applied in that frame, before the
        // `update` and the render that show it.
        let (runtime, trace) = RecordingRuntime::traced();
        let mut driver = PaneLoop::new(runtime);
        let mut out = Vec::new();
        driver.apply(
            create(CONSOLE, PaneKind::Console),
            &mut NoFrameSink,
            &mut out,
        );
        driver.view_mut(CONSOLE).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));

        // The order a click-and-type raises them in: focus, then the move that
        // decides what is under the pointer, then the press, then the keys.
        let sent = [
            PaneInput::Focus,
            PaneInput::MouseMove { x: 1, y: 1 },
            PaneInput::MouseDown { x: 1, y: 1 },
            PaneInput::KeyChar("a".to_string()),
        ];
        let mut queue: std::collections::VecDeque<PaneCommand> = sent
            .iter()
            .cloned()
            .map(|input| PaneCommand::Input { id: CONSOLE, input })
            .collect();

        trace.borrow_mut().clear();
        out.clear();
        let mut sink = RecordingSink::new();
        drain(&mut driver, &mut queue, &mut out);
        driver.iterate(&mut sink, &mut out);

        assert_eq!(
            driver.view_mut(CONSOLE).unwrap().inputs,
            sent.to_vec(),
            "the queue is a FIFO, and one pane's stream is its order"
        );
        assert_eq!(
            *trace.borrow(),
            vec![
                format!("input:{CONSOLE}:focus"),
                format!("input:{CONSOLE}:mousemove"),
                format!("input:{CONSOLE}:mousedown"),
                format!("input:{CONSOLE}:keychar"),
                "update".to_string(),
                "render".to_string(),
                format!("copy:{CONSOLE}"),
            ],
            "every input landed before the update and the render that show it"
        );
        assert_eq!(sink.published.len(), 1, "and this frame drew");
    }

    #[test]
    fn a_queued_gamepad_snapshot_is_in_the_slot_before_the_update_that_reads_it() {
        // The pad slot is filled by a system chained ahead of `drive_pane_host` and
        // drained with everything else, so it is in place for the pre-update
        // push — the phase whose whole reason is that a console polls the pads
        // inside `Renderer::update`.
        let (runtime, trace) = RecordingRuntime::traced();
        let mut driver = PaneLoop::new(runtime);
        let mut out = Vec::new();
        driver.apply(
            create(CONSOLE, PaneKind::Console),
            &mut NoFrameSink,
            &mut out,
        );
        driver.view_mut(CONSOLE).unwrap().paint = Some(FrameRect::full(WIDTH, HEIGHT));
        let mut sink = RecordingSink::new();
        // One iteration to settle the load edge: nothing is pushed into a
        // document that has not finished loading.
        driver.iterate(&mut sink, &mut out);

        let mut queue = std::collections::VecDeque::from([PaneCommand::SetGamepadScript(Some(
            "window.__phoenixSetGamepads([])".to_string(),
        ))]);
        trace.borrow_mut().clear();
        out.clear();
        drain(&mut driver, &mut queue, &mut out);
        driver.iterate(&mut sink, &mut out);

        assert_eq!(
            *trace.borrow(),
            vec![
                format!("push:{CONSOLE}"),
                "update".to_string(),
                "render".to_string(),
                format!("copy:{CONSOLE}"),
            ],
            "queued this frame, pushed this frame, and ahead of the update"
        );
    }

    #[test]
    fn a_resize_queued_after_an_input_is_applied_after_it() {
        // The two are raised by different systems, and the resize's system runs
        // first — but what decides which the view sees first is the queue, not
        // which system pushed. A resize that overtook an input would deliver a
        // click at coordinates the view had already moved past.
        let (runtime, trace) = RecordingRuntime::traced();
        let mut driver = PaneLoop::new(runtime);
        let mut out = Vec::new();
        driver.apply(
            create(CONSOLE, PaneKind::Console),
            &mut NoFrameSink,
            &mut out,
        );

        let mut queue = std::collections::VecDeque::from([
            PaneCommand::Input {
                id: CONSOLE,
                input: PaneInput::MouseMove { x: 2, y: 2 },
            },
            PaneCommand::Resize {
                id: CONSOLE,
                width: WIDTH * 2,
                height: HEIGHT,
                epoch: 3,
            },
        ]);
        trace.borrow_mut().clear();
        out.clear();
        drain(&mut driver, &mut queue, &mut out);

        assert_eq!(
            *trace.borrow(),
            vec![
                format!("input:{CONSOLE}:mousemove"),
                format!("resize:{CONSOLE}"),
            ]
        );
        assert_eq!(
            driver.view_mut(CONSOLE).unwrap().resizes,
            vec![(WIDTH * 2, HEIGHT)]
        );
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
