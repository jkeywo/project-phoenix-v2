//! Owned frames, bounded recycling and surface generations.
use crate::input_routing::PaneId;
use crate::surface_stats::FrameTrace;
use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
    sync::mpsc::{self, Receiver, Sender},
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PixelMode {
    OpaqueBgra,
    StraightRgba,
}
impl PixelMode {
    #[cfg(feature = "render")]
    pub const fn texture_format(self) -> bevy::render::render_resource::TextureFormat {
        match self {
            Self::StraightRgba => bevy::render::render_resource::TextureFormat::Rgba8UnormSrgb,
            Self::OpaqueBgra => bevy::render::render_resource::TextureFormat::Bgra8UnormSrgb,
        }
    }

    pub const fn fill(self) -> [u8; 4] {
        match self {
            Self::OpaqueBgra => [0, 0, 0, 255],
            Self::StraightRgba => [0, 0, 0, 0],
        }
    }
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
    pub fn with_trace(mut self, trace: Option<FrameTrace>) -> Self {
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
    pub frames: Vec<PaneFrame>,
    buffers_per_pane: usize,
    pub starved: usize,
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
    pub fn configure(&mut self, id: PaneId, pixels: PixelMode, len: usize) {
        self.return_staged();
        let (recycle, returned) = mpsc::channel();
        let fill = pixels.fill();
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
    /// Recycle an unpublished staged copy after a producer iteration.
    pub fn return_staged(&mut self) {
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
        self.frames.push(PaneFrame {
            id,
            epoch,
            rect,
            full,
            bytes: buffer,
        });
    }
    fn drop_pane(&mut self, id: PaneId) {
        self.return_staged();
        self.pools.remove(&id);
    }
}
