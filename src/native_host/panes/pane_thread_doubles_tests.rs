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
        PaneInput::WorkshopKey(_) => "workshop-key",
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
    pub visibility: Vec<bool>,
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
    fn set_visible(&mut self, visible: bool) {
        self.visibility.push(visible);
    }
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
