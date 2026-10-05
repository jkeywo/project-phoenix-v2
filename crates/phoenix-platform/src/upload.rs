//! A pane's frame reaches the GPU as a **sub-rectangle written into a texture
//! that stays put** (issue #1404, slice 1).
//!
//! # Why the texture is persistent
//!
//! Until this module existed, a pane frame was published by copying into the
//! `Image` asset's own `data` through `Assets::get_mut`, which raises an
//! `AssetEvent::Modified`. For the render world that event is not a hint: it
//! re-runs `prepare_assets::<GpuImage>`, which **re-creates the texture, its
//! view and its sampler**, and evicts every bind group keyed by that asset id
//! (`bevy_ui_render`'s `ImageNodeBindGroups`). Four panes meant four texture
//! re-creations and four bind-group rebuilds every frame, on the render thread,
//! for pixels that had already been copied once on the main thread.
//!
//! So the pane images are minted `RenderAssetUsages::RENDER_WORLD` — extracted
//! once, never written again from the main world — and a frame travels here
//! instead: as a [`PaneUpload`] carrying the pane's staging buffer and the
//! dirty rectangle inside it. [`upload_pane_frames`] turns that into one
//! `write_texture` against the texture bevy_ui is already sampling. Nothing is
//! invalidated, because nothing changed except texels.
//!
//! # The strided sub-rect
//!
//! The producer copies into a **full-size** buffer (`width * height * 4`,
//! stride `width * 4`) of which only `rect` is current, because that is what
//! vellum's `copy_rows_bgra` writes and re-packing the rect would cost a second
//! pass over the pixels. `write_texture` takes exactly that shape: a byte
//! `offset` to the rect's first pixel, a `bytes_per_row` of the whole surface's
//! stride, and an extent of the rect alone. [`upload_layout`] is that arithmetic
//! and every refusal wgpu would raise, kept pure so the arithmetic is checked
//! without a GPU — a bad layout is a validation panic in the render thread,
//! which is the one place in this process with nowhere to report a mistake.
//!
//! # The recycle-on-drop pool
//!
//! [`PaneFrameBuffer`] owns its `Vec<u8>` and returns it to the producer's free
//! list in `Drop`, over an `mpsc::Sender`. That is what makes the buffers a
//! pool rather than an allocation per frame *without* threading a lifetime
//! through the extract boundary: whoever ends up holding a frame last — the
//! render world after its `write_texture`, or a stale frame that was superseded
//! or refused — hands the allocation back by dropping it, and the producer
//! picks it up at the top of its next pass.
//!
//! # What guards a resize, in this slice
//!
//! Plainly: the **new `AssetId`**. A resize mints a whole new `Image` rather
//! than resizing the old one, so a frame produced against the old surface
//! carries the old id; its `gpu_images.get` misses (the old asset is gone, and
//! its `GpuImage` with it), it is deferred, and it ages out of
//! [`UPLOAD_DEFER_ATTEMPTS`]. [`PaneUpload::epoch`] and [`frame_is_current`] add
//! nothing to that today — they are carried for the pane thread (slice 5),
//! where frames cross a thread boundary and can outlive the bookkeeping that
//! made them, and where the id alone stops being enough.
//!
//! # Wholeness, and the one loss that is accepted
//!
//! A texture created or re-minted here starts as flat `pane_fill`, so the first
//! frame after it must cover the whole surface or the fill shows wherever the
//! page never repaints again. The producer asks for that with a forced copy and
//! clears its `needs_full` as soon as `copy_frame` returns a rectangle, so from
//! there on the wholeness lives in the frame — which is why a refused layout is
//! harmless (a refusal means the texture is a *different* one, which has its own
//! forced frame coming) and why [`defer`] promotes a partial newcomer that
//! supersedes a `full` deferral.
//!
//! One loss remains and is accepted: a `full` frame whose deferral exhausts
//! [`UPLOAD_DEFER_ATTEMPTS`] because its `GpuImage` never appeared. With no
//! `GpuImage` there is no texture for the fill to be wrong in — the image was
//! never prepared, and if it ever is, it is prepared from an asset that no
//! longer exists in the main world.
//!
//! # The Contract guard
//!
//! [`PaneUploadPlugin`] is registered unconditionally, including on a
//! `NativeRenderSurface::Contract` host, which has **no render sub-app** at all.
//! There, the extract and render systems do not exist, so a queued frame would
//! sit in [`PanePendingUploads`] for the life of the process holding its
//! buffer. The plugin's `Last` system drains it instead: no renderer means no
//! upload, and the honest thing to do with the frame is to let it go.

use bevy::asset::AssetId;
use bevy::image::Image;
use bevy::prelude::*;

use crate::surface_stats::{elapsed_ns, DiscardReason};
use std::time::Instant;
use vellum_ultralight::surface::DirtyRect;

/// The staging buffer a frame travels in.
///
/// Defined in [`crate::frames`] — the Bevy-free half of the seam, since
/// both the producer and this consumer carry one — and re-exported here, where
/// it was first introduced (issue #1404, slice 1) and where every existing
/// reader names it.
pub use crate::frames::PaneFrameBuffer;

/// How many render frames a deferred upload may wait for its `GpuImage` before
/// it is dropped.
///
/// An engine constant, not a tuning knob: a pane's image is extracted the frame
/// after it is added, so one or two attempts is the honest wait. Eight is slack
/// for a frame the asset system was busy on, and short enough that a frame for
/// an image that will never arrive (a pane closed the moment it opened) cannot
/// hold its buffer out of the pool for long.
pub const UPLOAD_DEFER_ATTEMPTS: u8 = 8;

/// Bytes per pixel of every pane texture — `Rgba8UnormSrgb` and
/// `Bgra8UnormSrgb` alike (`ultralight::pane_texture_format`).
const BYTES_PER_PIXEL: u32 = 4;

/// Where one dirty rectangle sits inside a full-size staging buffer, and the
/// texture region it is written to.
///
/// Exactly the three arguments `RenderQueue::write_texture` needs beyond the
/// texture itself: the destination `origin`, the copy `size`, and the source's
/// `offset`/`bytes_per_row`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UploadLayout {
    /// Top-left of the region in the destination texture, in texels.
    pub origin: (u32, u32),
    /// Width and height of the region, in texels.
    pub size: (u32, u32),
    /// Byte offset of the region's first pixel in the staging buffer.
    pub offset: u64,
    /// The staging buffer's row stride — the **surface's** width in bytes, not
    /// the rectangle's.
    pub bytes_per_row: u32,
}

/// The layout that uploads `rect` from a full-size `surface` buffer of
/// `buffer_len` bytes into a `texture`-sized texture, or `None` when wgpu would
/// refuse it.
///
/// Every refusal is a bookkeeping disagreement rather than a runtime condition:
/// a frame produced against a surface size the texture no longer has (a resize
/// in flight), a rectangle reported past the edge of its own surface, a buffer
/// too short for the rows it claims. Each is returned rather than panicked
/// because the caller is a render-world system and a skipped frame is a far
/// better failure than a validation abort.
pub fn upload_layout(
    rect: DirtyRect,
    surface: (u32, u32),
    texture: (u32, u32),
    buffer_len: usize,
) -> Option<UploadLayout> {
    if rect.is_empty() {
        return None;
    }
    // A frame is only ever written into the texture it was produced for. A
    // resize mints a NEW image, so a size disagreement means this frame belongs
    // to the previous one and its pixels are meaningless here.
    if surface != texture {
        return None;
    }
    let (width, height) = surface;
    if width == 0 || height == 0 {
        return None;
    }
    if rect.right > width || rect.bottom > height {
        return None;
    }
    let w = rect.right - rect.left;
    let h = rect.bottom - rect.top;
    let bytes_per_row = width.checked_mul(BYTES_PER_PIXEL)?;
    let offset = (u64::from(rect.top) * u64::from(width) + u64::from(rect.left))
        * u64::from(BYTES_PER_PIXEL);
    // The last row's end, not `offset + h * bpr`: the final row needs only its
    // own pixels, which is what makes a bottom-right rectangle fit a buffer
    // sized exactly `width * height * 4`.
    let needed = offset
        + u64::from(h - 1) * u64::from(bytes_per_row)
        + u64::from(w) * u64::from(BYTES_PER_PIXEL);
    if needed > buffer_len as u64 {
        return None;
    }
    Some(UploadLayout {
        origin: (rect.left, rect.top),
        size: (w, h),
        offset,
        bytes_per_row,
    })
}

/// One pane frame on its way to the GPU.
#[derive(Debug)]
pub struct PaneUpload {
    /// The pane texture this frame belongs to.
    pub image: AssetId<Image>,
    /// The pane's texture generation when this frame was copied, bumped by
    /// every resize.
    ///
    /// It is **not** what guards the resize race in this slice — the new
    /// `AssetId` a resize mints is (see the module note). It is carried for the
    /// pane thread of slice 5, where a frame crosses a thread boundary and the
    /// publisher checks it against the pane's current generation with
    /// [`frame_is_current`] before it ever reaches the render world.
    pub epoch: u64,
    /// The region of the surface that is current in [`bytes`](Self::bytes).
    pub rect: DirtyRect,
    /// The producing surface's size, which must equal the texture's.
    pub surface: (u32, u32),
    /// Whether the whole surface is current in [`bytes`](Self::bytes) — a
    /// forced copy, the first frame into a freshly minted texture.
    ///
    /// The upload itself does not care (the layout is the same either way), but
    /// two things read it: [`defer`] promotes a partial newcomer that supersedes
    /// a `full` deferral rather than losing the wholeness, and the render world
    /// counts these as
    /// [`PaneUploadTally::uploaded_full`] for `--frame-stats`.
    pub full: bool,
    /// The pane's full-size staging buffer, of which only `rect` is current.
    pub bytes: PaneFrameBuffer,
    /// How many render frames this upload has waited for its `GpuImage`.
    pub attempts: u8,
}

/// What one render frame's uploads did, counted for `--frame-stats`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PaneUploadTally {
    /// Frames written into a texture.
    pub uploaded: u32,
    /// How many of [`uploaded`](Self::uploaded) carried the whole surface — a
    /// forced copy, or a partial frame [`defer`] promoted. The upload twin of
    /// the producer's `forced` count: a `forced` with no matching
    /// `uploaded_full` is a whole-surface repaint that never landed.
    pub uploaded_full: u32,
    /// Bytes those writes moved (the rectangles' pixels, not the buffers').
    pub bytes: u64,
    /// Frames put aside because their `GpuImage` was not ready yet.
    pub deferred: u32,
    /// Frames discarded without reaching a texture — a refused layout, a
    /// superseded deferral, or one that ran out of [`UPLOAD_DEFER_ATTEMPTS`].
    pub dropped: u32,
}

/// The main world's outbox: frames the producer published this frame.
///
/// [`extract_pane_uploads`] empties it into the render world and leaves last
/// frame's [`tally`](Self::tally) behind in exchange, which is what
/// `frame_stats` reads.
#[derive(Resource, Default, Debug)]
pub struct PanePendingUploads {
    /// Frames published since the last extract.
    pub uploads: Vec<PaneUpload>,
    /// What the render world did with the previous batch.
    pub tally: PaneUploadTally,
}

/// The render world's inbox.
#[derive(Resource, Default, Debug)]
pub struct PaneUploadQueue {
    /// This frame's arrivals, moved wholesale from [`PanePendingUploads`].
    pub incoming: Vec<PaneUpload>,
    /// Frames whose `GpuImage` did not exist yet — at most one per image, the
    /// newest.
    pub deferred: Vec<PaneUpload>,
    /// This frame's counts, collected by the next extract.
    pub tally: PaneUploadTally,
    /// Whether a refused layout has already been warned about, so a persistent
    /// disagreement is one line rather than one per pane per frame.
    refused_warned: bool,
}

/// Put `upload` aside until its `GpuImage` exists, keeping at most one deferral
/// per image.
///
/// Newest wins: a pane that publishes every frame while its texture is being
/// created would otherwise pile up a frame per pane per frame, each holding a
/// buffer out of the pool. The superseded frame is counted as dropped, because
/// its dirty pixels are genuinely not going anywhere — and it is dropped here,
/// which is what returns its buffer.
///
/// # Inheriting a superseded `full`
///
/// The producer clears its `needs_full` the moment `copy_frame` hands back a
/// rectangle, so the wholeness of a frame lives only in the frame itself. If a
/// `full` deferral were replaced by an ordinary partial newcomer, nothing would
/// ever repaint the rest of a freshly minted texture and it would keep its
/// `pane_fill` wherever the page never repaints again. So a newcomer that
/// supersedes a `full` frame **inherits** its wholeness: its rect is widened to
/// the entire surface and its `full` set. That is sound because the newcomer's
/// buffer is the *same pane's pool buffer at the same size*, and a pool buffer
/// is seeded with the pane's own fill (see `pane_staging_pool`) before it ever
/// receives a rectangle: the pixels outside the newcomer's own dirty rectangle
/// are therefore either that pane's slightly older content or the fill the
/// texture already shows — never zeroes, and never another pane's pixels.
pub fn defer(deferred: &mut Vec<PaneUpload>, mut upload: PaneUpload, tally: &mut PaneUploadTally) {
    upload.attempts = upload.attempts.saturating_add(1);
    if upload.attempts >= UPLOAD_DEFER_ATTEMPTS {
        if let Some(trace) = upload.bytes.trace() {
            trace.deferred(upload.attempts);
        }
        tally.dropped += 1;
        if let Some(trace) = upload.bytes.trace_mut() {
            trace.discarded(DiscardReason::DeferralExhausted);
        }
        return;
    }
    if let Some(slot) = deferred.iter_mut().find(|held| held.image == upload.image) {
        // Carry the older frame's patience forward, so an image that never
        // arrives still ages out rather than being renewed by every new frame.
        upload.attempts = upload.attempts.max(slot.attempts);
        if let Some(trace) = upload.bytes.trace() {
            trace.deferred(upload.attempts);
        }
        if slot.full && !upload.full {
            upload.rect = DirtyRect {
                left: 0,
                top: 0,
                right: upload.surface.0,
                bottom: upload.surface.1,
            };
            upload.full = true;
            if let Some(trace) = upload.bytes.trace() {
                trace.promoted(u64::from(upload.surface.0) * u64::from(upload.surface.1));
            }
        }
        let mut superseded = std::mem::replace(slot, upload);
        if let Some(trace) = superseded.bytes.trace_mut() {
            trace.discarded(DiscardReason::SupersededDeferral);
        }
        drop(superseded);
        tally.dropped += 1;
    } else {
        if let Some(trace) = upload.bytes.trace() {
            trace.deferred(upload.attempts);
        }
        deferred.push(upload);
    }
    tally.deferred += 1;
}

/// Whether a frame produced at `epoch` is still the pane's `current`
/// generation.
///
/// A frame from a previous generation cannot describe a newly minted texture.
/// The live main-world acceptance gate is `PaneMirror::accepts_frame`, which
/// also refuses frames for closed panes before they enter the upload queue.
pub fn frame_is_current(epoch: u64, current: u64) -> bool {
    epoch == current
}

/// Installs the pane-frame upload path, in whichever half of the app exists.
///
/// Registered unconditionally — including with the `ultralight` feature off and
/// on a renderer-less Contract host — so that the resource the producer
/// publishes into is always there and nothing can accumulate where there is no
/// GPU. See the module note's "Contract guard".
pub struct PaneUploadPlugin;

impl Plugin for PaneUploadPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PanePendingUploads>();

        use bevy::render::{Render, RenderApp, RenderSystems};

        if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
            render_app
                .init_resource::<PaneUploadQueue>()
                .add_systems(ExtractSchedule, extract_pane_uploads)
                .add_systems(
                    Render,
                    // AFTER `prepare_assets::<GpuImage>` (`PrepareAssets`) so a
                    // pane image added this frame already has its texture, and
                    // BEFORE `PrepareBindGroups` and the draw — the texels are
                    // in place by the time anything samples them.
                    upload_pane_frames.in_set(RenderSystems::PrepareResources),
                );
        } else {
            app.add_systems(Last, discard_pane_uploads);
        }
    }
}

/// Move the main world's published frames into the render world, and last
/// frame's counts back.
///
/// `ResMut<MainWorld>` rather than `Extract<ResMut<_>>` because `Extract`'s
/// parameter must be read-only (`bevy_render`'s `extract_param`), and this
/// needs to *empty* the main-world resource — the same shape `bevy_render`'s
/// own `extract_render_asset` uses. Both halves are pointer moves: extract runs
/// while the main thread is stalled, and no byte of a frame is touched here.
pub fn extract_pane_uploads(
    mut main: ResMut<bevy::render::MainWorld>,
    mut queue: ResMut<PaneUploadQueue>,
) {
    let Some(mut pending) = main.get_resource_mut::<PanePendingUploads>() else {
        return;
    };
    for upload in &pending.uploads {
        if let Some(trace) = upload.bytes.trace() {
            trace.extracted();
        }
    }
    queue.incoming.append(&mut pending.uploads);
    pending.tally = std::mem::take(&mut queue.tally);
}

/// Write every arrived frame's dirty rectangle into its pane texture.
pub fn upload_pane_frames(
    mut queue: ResMut<PaneUploadQueue>,
    gpu_images: Res<bevy::render::render_asset::RenderAssets<bevy::render::texture::GpuImage>>,
    render_queue: Res<bevy::render::renderer::RenderQueue>,
) {
    use bevy::render::render_resource::{
        Extent3d, Origin3d, TexelCopyBufferLayout, TexelCopyTextureInfo, TextureAspect,
    };

    if queue.incoming.is_empty() && queue.deferred.is_empty() {
        return;
    }
    // The deferrals first, so that where an image has both, this frame's newer
    // pixels land last.
    let mut batch = std::mem::take(&mut queue.deferred);
    let mut incoming = std::mem::take(&mut queue.incoming);
    batch.append(&mut incoming);

    let mut tally = std::mem::take(&mut queue.tally);
    let mut deferred = Vec::new();
    let mut warn_refusal = false;
    for mut upload in batch {
        let Some(gpu) = gpu_images.get(upload.image) else {
            defer(&mut deferred, upload, &mut tally);
            continue;
        };
        let texture = (gpu.size.width, gpu.size.height);
        let Some(layout) = upload_layout(upload.rect, upload.surface, texture, upload.bytes.len())
        else {
            tally.dropped += 1;
            if let Some(trace) = upload.bytes.trace_mut() {
                trace.discarded(DiscardReason::RefusedLayout);
            }
            warn_refusal = true;
            // Dropped here, which recycles the buffer.
            continue;
        };
        let write_started = upload.bytes.trace().map(|_| Instant::now());
        render_queue.write_texture(
            TexelCopyTextureInfo {
                texture: &gpu.texture,
                mip_level: 0,
                origin: Origin3d {
                    x: layout.origin.0,
                    y: layout.origin.1,
                    z: 0,
                },
                aspect: TextureAspect::All,
            },
            &upload.bytes,
            TexelCopyBufferLayout {
                offset: layout.offset,
                bytes_per_row: Some(layout.bytes_per_row),
                rows_per_image: None,
            },
            Extent3d {
                width: layout.size.0,
                height: layout.size.1,
                depth_or_array_layers: 1,
            },
        );
        let write_texture_ns = elapsed_ns(write_started);
        if let Some(trace) = upload.bytes.trace_mut() {
            trace.uploaded(
                u64::from(layout.size.0) * u64::from(layout.size.1),
                upload.full,
                write_texture_ns,
            );
        }
        tally.uploaded += 1;
        if upload.full {
            tally.uploaded_full += 1;
        }
        tally.bytes +=
            u64::from(layout.size.0) * u64::from(layout.size.1) * u64::from(BYTES_PER_PIXEL);
    }
    queue.deferred = deferred;
    queue.tally = tally;
    if warn_refusal && !queue.refused_warned {
        queue.refused_warned = true;
        // The render world holds no `LogFilterConfig` (it is a main-world
        // resource), so this is the sanctioned bare form — AGENTS.md's second
        // logging rule.
        warn!(
            target: "lobby",
            "pane host: a pane frame did not match its texture and was dropped; \
             this is reported once per run"
        );
    }
}

/// The renderer-less host's drain: nothing can upload, so nothing is kept.
pub fn discard_pane_uploads(mut pending: ResMut<PanePendingUploads>) {
    if !pending.uploads.is_empty() {
        let dropped = pending.uploads.len() as u32;
        for upload in &mut pending.uploads {
            if let Some(trace) = upload.bytes.trace_mut() {
                trace.discarded(DiscardReason::NoRenderer);
            }
        }
        pending.uploads.clear();
        pending.tally = PaneUploadTally {
            dropped,
            ..PaneUploadTally::default()
        };
    }
}

#[cfg(test)]
#[path = "upload_tests.rs"]
mod tests;
