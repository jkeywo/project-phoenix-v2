use super::*;
use std::sync::mpsc::channel;

use crate::input_routing::PaneId;

const PANE: PaneId = PaneId(7);

fn rect(left: u32, top: u32, right: u32, bottom: u32) -> DirtyRect {
    DirtyRect {
        left,
        top,
        right,
        bottom,
    }
}

fn buffer(len: usize) -> PaneFrameBuffer {
    PaneFrameBuffer::new(PANE, vec![0; len], None)
}

fn upload(image: AssetId<Image>, epoch: u64, len: usize) -> PaneUpload {
    PaneUpload {
        image,
        epoch,
        rect: rect(0, 0, 4, 4),
        surface: (4, 4),
        full: true,
        bytes: buffer(len),
        attempts: 0,
    }
}

fn observed_upload(observer: &crate::surface_stats::SurfaceObserver, full: bool) -> PaneUpload {
    use crate::surface_stats::{FullCopyReasons, SurfaceIdentity};
    let mut upload = upload(AssetId::<Image>::invalid(), 2, 64);
    upload.full = full;
    if !full {
        upload.rect = rect(1, 1, 2, 2);
    }
    let identity = SurfaceIdentity {
        id: PANE.0,
        epoch: 2,
        kind: "console",
        width: 4,
        height: 4,
        device_scale: 2.0,
        visible: true,
    };
    upload.bytes = upload.bytes.with_trace(Some(observer.produced(
        identity,
        upload.rect.pixel_count(),
        full,
        FullCopyReasons::default(),
        None,
    )));
    upload
}

#[test]
fn attribution_records_supersession_full_promotion_and_inherited_deferral_age() {
    use crate::surface_stats::{Operation, SurfaceObserver};
    let observer = SurfaceObserver::new(Instant::now(), 64);
    let mut deferred = Vec::new();
    let mut tally = PaneUploadTally::default();
    let mut first = observed_upload(&observer, true);
    first.attempts = 3;
    defer(&mut deferred, first, &mut tally);
    let next = observed_upload(&observer, false);
    defer(&mut deferred, next, &mut tally);
    assert_eq!(deferred.len(), 1);
    assert_eq!(deferred[0].attempts, 4);
    assert!(deferred[0].full);
    let events = observer.events();
    assert!(events
        .iter()
        .any(|e| e.frame == Some(1)
            && matches!(e.operation, Operation::Deferred { attempts: 4, .. })));
    assert!(events.iter().any(
        |e| e.frame == Some(1) && matches!(e.operation, Operation::PromotedFull { pixels: 16 })
    ));
    assert!(events.iter().any(|e| e.frame == Some(0)
        && matches!(
            e.operation,
            Operation::Discarded {
                reason: DiscardReason::SupersededDeferral,
                ..
            }
        )));
    let mut last = deferred.pop().unwrap();
    last.attempts = UPLOAD_DEFER_ATTEMPTS - 1;
    defer(&mut deferred, last, &mut tally);
    assert!(deferred.is_empty());
    let events = observer.events();
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e.operation, Operation::Discarded { .. }))
            .count(),
        2
    );
    assert!(matches!(
        events.last().unwrap().operation,
        Operation::Discarded {
            reason: DiscardReason::DeferralExhausted,
            ..
        }
    ));
}

#[test]
fn attribution_marks_contract_disposal_before_buffer_recycling() {
    use crate::surface_stats::{Operation, SurfaceObserver};
    let observer = SurfaceObserver::new(Instant::now(), 16);
    let mut app = App::new();
    app.add_plugins(PaneUploadPlugin);
    app.world_mut()
        .resource_mut::<PanePendingUploads>()
        .uploads
        .push(observed_upload(&observer, true));
    app.update();
    app.update();
    let events = observer.events();
    assert_eq!(events.len(), 2);
    assert!(matches!(
        events[1].operation,
        Operation::Discarded {
            reason: DiscardReason::NoRenderer,
            ..
        }
    ));
    assert_eq!(events[1].surface.unwrap().epoch, 2);
}

#[test]
fn a_full_rect_uploads_the_whole_buffer_from_its_start() {
    let layout = upload_layout(rect(0, 0, 64, 64), (64, 64), (64, 64), 64 * 64 * 4).unwrap();
    assert_eq!(
        layout,
        UploadLayout {
            origin: (0, 0),
            size: (64, 64),
            offset: 0,
            bytes_per_row: 256,
        }
    );
}

#[test]
fn a_sub_rect_keeps_the_surfaces_stride_and_offsets_to_its_first_pixel() {
    let layout = upload_layout(rect(16, 8, 48, 24), (64, 64), (64, 64), 64 * 64 * 4).unwrap();
    assert_eq!(layout.origin, (16, 8));
    assert_eq!(layout.size, (32, 16));
    assert_eq!(layout.offset, (8 * 64 + 16) * 4);
    // The SURFACE's stride, not the rectangle's — that is the whole point.
    assert_eq!(layout.bytes_per_row, 256);
}

#[test]
fn a_bottom_right_rect_fits_a_buffer_sized_exactly_for_the_surface() {
    let len = 64 * 48 * 4;
    let layout = upload_layout(rect(60, 44, 64, 48), (64, 48), (64, 48), len).unwrap();
    assert_eq!(layout.origin, (60, 44));
    assert_eq!(layout.size, (4, 4));
    assert_eq!(layout.offset, (44 * 64 + 60) * 4);
    // One byte short and it must be refused, which is the check that the
    // last row is measured by its own pixels rather than a whole stride.
    assert!(upload_layout(rect(60, 44, 64, 48), (64, 48), (64, 48), len - 1).is_none());
}

#[test]
fn an_empty_or_degenerate_rect_uploads_nothing() {
    let len = 64 * 64 * 4;
    for empty in [
        DirtyRect::empty(),
        rect(10, 10, 10, 20),
        rect(10, 10, 20, 10),
        rect(20, 10, 10, 20),
    ] {
        assert!(upload_layout(empty, (64, 64), (64, 64), len).is_none());
    }
}

#[test]
fn a_stale_surface_or_a_rect_past_the_edge_is_refused() {
    let len = 64 * 64 * 4;
    // The texture was re-minted at another size: this frame is the previous
    // generation's and its pixels mean nothing here.
    assert!(upload_layout(rect(0, 0, 64, 64), (64, 64), (32, 32), len).is_none());
    assert!(upload_layout(rect(0, 0, 32, 32), (32, 32), (64, 64), len).is_none());
    // Bounds reported from a size the view had a moment ago.
    assert!(upload_layout(rect(0, 0, 65, 64), (64, 64), (64, 64), len).is_none());
    assert!(upload_layout(rect(0, 0, 64, 65), (64, 64), (64, 64), len).is_none());
    // A zero-sized surface has no stride at all.
    assert!(upload_layout(rect(0, 0, 1, 1), (0, 0), (0, 0), len).is_none());
}

#[test]
fn a_buffer_too_short_for_the_rows_it_claims_is_refused() {
    assert!(upload_layout(rect(0, 0, 64, 64), (64, 64), (64, 64), 64 * 64 * 4 - 1).is_none());
    assert!(upload_layout(rect(0, 0, 64, 64), (64, 64), (64, 64), 0).is_none());
    // A full-size buffer is enough for any rect inside it.
    assert!(upload_layout(rect(1, 1, 63, 63), (64, 64), (64, 64), 64 * 64 * 4).is_some());
}

#[test]
fn dropping_a_frame_buffer_returns_its_allocation_to_the_pool() {
    let (tx, rx) = channel();
    {
        let mut held = PaneFrameBuffer::new(PANE, vec![9; 16], Some(tx));
        assert_eq!(held.len(), 16);
        held[0] = 1;
        assert_eq!(held[0], 1);
        assert_eq!(held.pane(), PANE);
    }
    let (pane, bytes) = rx.try_recv().expect("the buffer comes back on drop");
    assert_eq!(pane, PANE);
    assert_eq!(bytes.len(), 16);
    assert!(rx.try_recv().is_err());
}

#[test]
fn a_buffer_with_no_sender_simply_frees() {
    let held = PaneFrameBuffer::new(PANE, vec![0; 4], None);
    assert_eq!(held.len(), 4);
    drop(held);
}

#[test]
fn each_panes_buffers_come_back_to_that_panes_pool() {
    let (tx, rx) = channel();
    let first = PaneId(1);
    let second = PaneId(2);
    drop(PaneFrameBuffer::new(first, vec![0; 8], Some(tx.clone())));
    drop(PaneFrameBuffer::new(second, vec![0; 12], Some(tx)));
    let routed: Vec<_> = rx.try_iter().map(|(id, bytes)| (id, bytes.len())).collect();
    assert_eq!(routed, vec![(first, 8), (second, 12)]);
}

#[test]
fn a_deferred_image_keeps_only_its_newest_frame_and_recycles_the_rest() {
    let (tx, rx) = channel();
    let image = AssetId::<Image>::invalid();
    let mut deferred = Vec::new();
    let mut tally = PaneUploadTally::default();

    let mut first = upload(image, 1, 8);
    first.bytes = PaneFrameBuffer::new(PANE, vec![0; 8], Some(tx.clone()));
    defer(&mut deferred, first, &mut tally);
    assert_eq!(deferred.len(), 1);
    assert_eq!(tally.deferred, 1);
    assert_eq!(tally.dropped, 0);
    assert!(rx.try_recv().is_err(), "the held frame keeps its buffer");

    let mut second = upload(image, 2, 8);
    second.bytes = PaneFrameBuffer::new(PANE, vec![0; 8], Some(tx));
    defer(&mut deferred, second, &mut tally);
    assert_eq!(deferred.len(), 1, "one deferral per image, the newest");
    assert_eq!(deferred[0].epoch, 2);
    assert_eq!(tally.deferred, 2);
    assert_eq!(tally.dropped, 1, "the superseded frame is a loss");
    assert!(
        rx.try_recv().is_ok(),
        "the superseded frame's buffer goes back to the pool"
    );
}

#[test]
fn a_partial_frame_superseding_a_full_one_inherits_its_whole_surface() {
    let image = AssetId::<Image>::invalid();
    let mut deferred = Vec::new();
    let mut tally = PaneUploadTally::default();

    // A forced frame — the first into a freshly minted texture — is put
    // aside because its `GpuImage` is not there yet.
    defer(&mut deferred, upload(image, 1, 4 * 4 * 4), &mut tally);

    // The next frame is an ordinary partial repaint of one corner. It must
    // not simply replace the full frame, or the rest of the texture would
    // keep its fill for as long as the page does not repaint it.
    let mut partial = upload(image, 1, 4 * 4 * 4);
    partial.rect = rect(2, 2, 3, 3);
    partial.full = false;
    defer(&mut deferred, partial, &mut tally);

    assert_eq!(deferred.len(), 1);
    assert!(deferred[0].full, "the wholeness is inherited, not lost");
    assert_eq!(
        deferred[0].rect,
        rect(0, 0, 4, 4),
        "and the rect is widened to the surface it is whole for"
    );

    // A partial superseding a partial stays exactly as it came.
    let mut later = upload(image, 1, 4 * 4 * 4);
    later.rect = rect(1, 0, 2, 1);
    later.full = false;
    let mut plain = Vec::new();
    let mut first = upload(image, 1, 4 * 4 * 4);
    first.full = false;
    first.rect = rect(0, 0, 1, 1);
    defer(&mut plain, first, &mut tally);
    defer(&mut plain, later, &mut tally);
    assert!(!plain[0].full);
    assert_eq!(plain[0].rect, rect(1, 0, 2, 1));
}

#[test]
fn a_deferral_that_never_finds_its_texture_ages_out() {
    let image = AssetId::<Image>::invalid();
    let mut deferred = Vec::new();
    let mut tally = PaneUploadTally::default();
    for _ in 0..UPLOAD_DEFER_ATTEMPTS {
        let held = deferred.pop().unwrap_or_else(|| upload(image, 1, 8));
        defer(&mut deferred, held, &mut tally);
    }
    assert!(
        deferred.is_empty(),
        "it is dropped rather than held forever"
    );
    assert_eq!(tally.dropped, 1);
    assert_eq!(u32::from(UPLOAD_DEFER_ATTEMPTS), tally.deferred + 1);
}

#[test]
fn a_frame_from_a_previous_image_generation_is_not_current() {
    assert!(frame_is_current(4, 4));
    assert!(!frame_is_current(3, 4));
    assert!(!frame_is_current(5, 4));
}

#[test]
fn a_renderless_host_drains_what_it_cannot_upload() {
    let image = AssetId::<Image>::invalid();
    let mut app = App::new();
    app.add_plugins(PaneUploadPlugin);
    app.world_mut()
        .resource_mut::<PanePendingUploads>()
        .uploads
        .push(upload(image, 1, 64));
    app.update();
    let pending = app.world().resource::<PanePendingUploads>();
    assert!(pending.uploads.is_empty());
    assert_eq!(pending.tally.dropped, 1);
}
