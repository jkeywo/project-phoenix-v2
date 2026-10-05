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

#[cfg(test)]
use std::collections::HashMap;
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
    elapsed_ns, CopyObservation, FullCopyReasons, Operation, SurfaceIdentity, SurfaceObserver,
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
    /// Offline Authoring capability; no crew or GM identity.
    Workshop,
    /// Privileged local GM; uses a private bridge and no crew session.
    GameMaster,
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
    pub const fn pixel_mode(self) -> PixelMode {
        if self.transparent() {
            PixelMode::StraightRgba
        } else {
            PixelMode::OpaqueBgra
        }
    }

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
    WorkshopKey(crate::native_host::workshop::keyboard::WorkshopKey),
    Focus,
    Unfocus,
}

/// Pixels one wheel notch (one "line" of scroll) moves a pane's page.
///
/// A wheel reports its travel in lines, not pixels — a notch is `1.0` — and
/// the view scrolls by pixel, so forwarding the raw line count scrolled the GM
/// console ONE PIXEL per notch (GM console feedback: "scrolling the panel on
/// the right with the mouse wheel is very slow"). Chrome moves 100px a notch
/// on Windows and Firefox three lines of ~19px; this sits between them so a
/// desk panel of a few hundred pixels crosses in a handful of notches.
pub const WHEEL_LINE_PIXELS: f32 = 60.0;

impl PaneInput {
    /// The [`PaneInput::Scroll`] a wheel report becomes. `line_units` says the
    /// deltas count notches rather than pixels (a touchpad reports pixels and
    /// is passed through unchanged); rounding rather than truncating keeps a
    /// sub-notch tick from vanishing to zero.
    pub fn scroll_from_wheel(line_units: bool, dx: f32, dy: f32) -> Self {
        let scale = if line_units { WHEEL_LINE_PIXELS } else { 1.0 };
        PaneInput::Scroll {
            dx: (dx * scale).round() as i32,
            dy: (dy * scale).round() as i32,
        }
    }
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

    /// Suspend painting of an uncomposited document, without unloading it or
    /// dropping its reliable bridge messages. Loading views retain the request.
    fn set_visible(&mut self, visible: bool);

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

/// Owned configuration; the runtime is constructed on its owning thread.
pub struct PaneThreadConfig {
    pub bus: Option<PaneBus>,
    pub lobby: Option<HostLobbyBridge>,
    pub gm: Option<crate::native_host::native_gm::bridge::NativeGmBridge>,
    pub audio_visuals: Option<crate::native_host::audio::visual::NativeAudioVisual>,
    pub workshop: Option<crate::native_host::workshop::bridge::WorkshopBridge>,
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
            gm: None,
            audio_visuals: None,
            workshop: None,
            period: Duration::from_millis(16),
            buffers_per_pane: PANE_STAGING_BUFFERS,
            measure: false,
            observer: None,
            on_shutdown_timeout: None,
        }
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
    let mut out = Vec::new();
    let stop = driver.apply(command, sink, &mut out) == LoopControl::Stop;
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
                driver.gm = config.gm;
                driver.audio_visuals = config.audio_visuals;
                driver.workshop = config.workshop;
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
                    out.extend(sink.frames.drain(..).map(PaneEvent::Frame));
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
    audio_reader: crate::native_host::audio::visual::VisualReader,
}

impl<V> LoopPane<V> {
    fn identity(&self) -> SurfaceIdentity {
        SurfaceIdentity {
            id: self.id.0,
            epoch: self.epoch,
            kind: match self.kind {
                PaneKind::GameMaster => "gm",
                PaneKind::Workshop => "workshop",
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
    gm: Option<crate::native_host::native_gm::bridge::NativeGmBridge>,
    audio_visuals: Option<crate::native_host::audio::visual::NativeAudioVisual>,
    workshop: Option<crate::native_host::workshop::bridge::WorkshopBridge>,
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
            gm: None,
            audio_visuals: None,
            workshop: None,
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
                    Ok(mut view) => {
                        view.set_visible(visible);
                        sink.configure(
                            id,
                            kind.pixel_mode(),
                            spec.width as usize * spec.height as usize * 4,
                        );
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
                            audio_reader: Default::default(),
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
                    sink.configure(
                        id,
                        pane.kind.pixel_mode(),
                        width as usize * height as usize * 4,
                    );
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
                    pane.view.set_visible(visible);
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
            PaneCommand::SetGamepadScript(script) => {
                if let (Some(bus), Some(script)) = (&self.bus, &script) {
                    bus.observe_gamepads(script);
                }
                self.gamepad_script = script;
            }
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
            gm,
            audio_visuals,
            workshop,
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
                    let filtered = bus
                        .as_ref()
                        .map(|bus| bus.gamepads_for_pane(pane.id, script));
                    let applied = pane
                        .view
                        .push(filtered.as_deref().unwrap_or(script))
                        .is_ok();
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
                PaneKind::Workshop => {
                    if let Some(bridge) = &*workshop {
                        let count = bridge.pump(pane.id, &mut pane.view);
                        pane.pushed_this_iteration = count > 0;
                        applied = count as u64;
                    }
                }
                PaneKind::GameMaster => {
                    if let Some(bridge) = &*gm {
                        let count = bridge.pump(pane.id, &mut pane.view);
                        pane.pushed_this_iteration = count > 0;
                        applied = count as u64;
                    }
                }
                // The HUD overlay is driven by the host's own readout, held in
                // the slot until replaced. Apply a revision once successfully
                // per loaded view, or again when its lifecycle owes a refresh.
                // This runs before render so the new DOM is painted this pass;
                // a quiet revision leaves animation dirty detection intact.
                PaneKind::Hud => {
                    if let Some(audio) = &*audio_visuals {
                        // Current equivalents are observed here, never queued in
                        // the retained HUD script. New/hidden/loading documents
                        // seed silently; failed pushes consume the occurrence.
                        for cue in pane.audio_reader.read(
                            audio,
                            pane.view.is_ready(),
                            pane.visible,
                            Instant::now(),
                        ) {
                            if let Ok(json) = crate::core::codec::encode_native_audio_visual(&cue) {
                                let script = vellum_ultralight::bridge::push_call(
                                    "window.__phoenixHudAudioCue",
                                    &json,
                                );
                                if pane.view.push(&script).is_ok() {
                                    pane.pushed_this_iteration = true;
                                    applied += 1;
                                } else {
                                    failed += 1;
                                    if matches!(
                                        cue,
                                        crate::native_host::audio::visual::VisualCue::Lifecycle { .. }
                                            | crate::native_host::audio::visual::VisualCue::Beam { .. }
                                    ) {
                                        pane.audio_reader.retry_current();
                                    }
                                }
                            }
                        }
                    }
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
                            applied += 1;
                        } else {
                            failed += 1;
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
#[path = "pane_thread_wheel_tests.rs"]
mod wheel_tests;

#[cfg(test)]
#[path = "pane_thread_thread_tests.rs"]
mod thread_tests;

#[cfg(test)]
#[path = "pane_thread_doubles_tests.rs"]
pub(crate) mod doubles;

#[cfg(test)]
#[path = "pane_thread_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "pane_thread_loop_tests.rs"]
mod loop_tests;

pub use phoenix_platform::frames::*;
