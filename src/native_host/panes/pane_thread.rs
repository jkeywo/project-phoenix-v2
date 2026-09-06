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

use super::registry::PaneId;
use super::surface::{PaneSurface, PaneSurfaceError};
use super::transport::PaneInputRefusal;

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
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PaneThreadSample {
    /// `Renderer::update` — the library's own timers, network and script work.
    pub update_ms: f64,
    /// Messages moved both ways across the bridge, for every pane.
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
    fn stage(&mut self, id: PaneId, len: usize) -> Option<&mut [u8]>;

    /// Publish the staged buffer as a frame at `epoch` covering `rect`.
    fn publish(&mut self, id: PaneId, epoch: u64, rect: FrameRect);

    /// Forget everything held for a pane that has gone.
    fn drop_pane(&mut self, id: PaneId);
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
    }

    impl PaneRuntime for RecordingRuntime {
        type View = RecordingView;

        fn update(&mut self) {
            self.phases.push(RuntimePhase::Update);
        }

        fn render(&mut self) {
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
            let mut view = RecordingView::ready();
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
